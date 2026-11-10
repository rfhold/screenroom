#!/usr/bin/env bash
set -euo pipefail

category=${E2E_CATEGORY-all}
if [[ ${E2E_CATEGORY+x} == x && -z "$E2E_CATEGORY" ]]; then
    printf 'E2E_CATEGORY must not be empty\n' >&2; exit 2
fi
if [[ ${E2E_TEST+x} == x ]]; then
    case "$E2E_TEST" in
        fault_recovery) legacy=retry ;;
        home) legacy=all ;;
        *) printf 'Unknown E2E_TEST: %s\n' "$E2E_TEST" >&2; exit 2 ;;
    esac
    if [[ ${E2E_CATEGORY+x} == x && "$category" != "$legacy" ]]; then
        printf 'Conflicting E2E_CATEGORY and legacy E2E_TEST\n' >&2; exit 2
    fi
    category=$legacy
fi
case "$category" in
    happy-path) scenarios=(happy_path) ;;
    retry) scenarios=(fault_recovery) ;;
    regression) scenarios=(regression_catalog regression_search) ;;
    all) scenarios=(happy_path fault_recovery regression_catalog regression_search) ;;
    *) printf 'Unknown E2E_CATEGORY: %s\n' "$category" >&2; exit 2 ;;
esac

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
output=${1:-"$root/build/e2e"}
mkdir -p "$output"
output=$(cd -- "$output" && pwd)
image=screenroom-linux-e2e:local
repo=$(cd "$root/../.." && pwd)
export SCREENROOM_BACKEND_DESCRIPTOR=${SCREENROOM_BACKEND_DESCRIPTOR:-$repo/build/backend-runtime/screenroom-e2e/descriptor.json}
export SCREENROOM_BACKEND_ATTACH=1
bash "$repo/scripts/backend.sh" status
bash "$repo/scripts/backend.sh" wait
read -r network upstream instance origin user < <(python3 - "$SCREENROOM_BACKEND_DESCRIPTOR" <<'PY'
import json,sys
d=json.load(open(sys.argv[1])); assert d['ready']
print(d['clientNetwork'],d['clientUpstream'],d['instanceID'],d['publicOrigin'],d['userID'])
PY
)
for alias in SCREENROOM_API_ENDPOINT SCREENROOM_ENDPOINT SCREENROOM_E2E_API_ENDPOINT; do
    if [[ -n ${!alias:-} && ${!alias} != "$origin" ]]; then printf '%s conflicts with shared backend origin\n' "$alias" >&2; exit 2; fi
done
SCREENROOM_API_ENDPOINT="$origin" python3 "$root/tests/e2e/origin.py" >/dev/null
if [[ -n ${SCREENROOM_BACKEND_INSTANCE_ID:-} && $SCREENROOM_BACKEND_INSTANCE_ID != "$instance" ]]; then printf 'Backend instance mismatch\n' >&2; exit 2; fi
container=
namespace=${SCREENROOM_E2E_SETTINGS_NAMESPACE:-${E2E_PROFILE_PREFIX:-linux-$(date +%s)-$$}}
if [[ ! $namespace =~ ^[a-zA-Z0-9_-]+$ ]]; then printf 'Invalid local settings namespace\n' >&2; exit 2; fi
screenshots=()
cleanup() {
    local status=$?
    if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null || true; fi
    if [[ "$status" != 0 ]]; then rm -f "${screenshots[@]}"; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

# A failed rerun must not retain earlier success screenshots.
screenshots=("$output"/{home,sidebar,library,search,detail}.png)
screenshots+=("$output/happy_path"/{home,sidebar,library,search,detail}.png)
rm -f "${screenshots[@]}"
DOCKER_BUILDKIT=1 docker build --progress=plain -f "$root/tests/e2e/Dockerfile" -t "$image" "$root/../.." >"$output/build.log" 2>&1 || {
    cat "$output/build.log"
    exit 1
}
: >"$output/e2e.log"
status=0
for scenario in "${scenarios[@]}"; do
    destination="$output"
    if [[ ${#scenarios[@]} -gt 1 ]]; then
        destination="$output/$scenario"
        mkdir -p "$destination"
    fi
    container=$(docker create --network "$network" --cap-drop ALL --security-opt no-new-privileges \
        -e SCREENROOM_API_ENDPOINT="$origin" -e SCREENROOM_USER_ID="${SCREENROOM_USER_ID:-$user}" \
        -e SCREENROOM_BACKEND_INSTANCE_ID="$instance" -e SCREENROOM_BACKEND_UPSTREAM="$upstream" \
        -e SCREENROOM_E2E_SETTINGS_NAMESPACE="$namespace-$scenario" \
        -e E2E_REQUIRED_LABEL -e E2E_RESOLUTION -e E2E_APP -e E2E_SCENARIO="$scenario" \
        --pids-limit 128 --memory 2g "$image")
    docker start -a "$container" >"$output/current-scenario.log" 2>&1 || status=$?
    docker cp "$container:/artifacts/." "$destination/"
    cat "$output/current-scenario.log" >>"$output/e2e.log"
    if [[ "$destination" != "$output" ]]; then
        cp "$output/current-scenario.log" "$destination/e2e.log"
        if [[ "$scenario" == happy_path && "$status" == 0 ]]; then
            cp "$destination"/{home,sidebar,library,search,detail}.png "$output/"
        fi
    fi
    docker rm "$container" >/dev/null
    container=
done
rm -f "$output/current-scenario.log"
if [[ "$status" != 0 ]]; then rm -f "${screenshots[@]}"; fi
cat "$output/e2e.log"
printf 'Artifacts: %s\n' "$output"
exit "$status"
