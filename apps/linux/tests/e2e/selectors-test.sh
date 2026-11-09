#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
output=${1:?Usage: selectors-test.sh OUTPUT SEED_PNG_DIRECTORY}
seeds=${2:?Usage: selectors-test.sh OUTPUT SEED_PNG_DIRECTORY}
mkdir -p "$output"
output=$(cd -- "$output" && pwd)
seeds=$(cd -- "$seeds" && pwd)
mkdir -p "$output/bin"
cat >"$output/bin/docker" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$DOCKER_SENTINEL"
exit 97
SH
chmod +x "$output/bin/docker"
export PATH="$output/bin:$PATH"
export DOCKER_SENTINEL="$output/docker-invocations.log"
rm -f "$DOCKER_SENTINEL"

reject() {
    local name=$1 rc=0
    shift
    local destination="$output/$name"
    mkdir -p "$destination"
    cp "$seeds"/{home,sidebar,library,search,detail}.png "$destination/"
    (cd "$destination" && shasum ./*.png) >"$destination/before.sha"
    env -u E2E_CATEGORY -u E2E_TEST "$@" bash "$root/tests/e2e/run.sh" "$destination" >"$destination/selector.log" 2>&1 || rc=$?
    [[ "$rc" == 2 ]]
    (cd "$destination" && shasum ./*.png) >"$destination/after.sha"
    cmp "$destination/before.sha" "$destination/after.sha"
    [[ ! -e "$destination/build.log" && ! -e "$DOCKER_SENTINEL" ]]
    printf '%s: exit 2; five PNG hashes unchanged; no Docker invocation\n' "$name"
}

reject empty-category E2E_CATEGORY=
reject empty-legacy E2E_TEST=
reject both-empty E2E_CATEGORY= E2E_TEST=
reject empty-category-with-legacy E2E_CATEGORY= E2E_TEST=fault_recovery
reject category-with-empty-legacy E2E_CATEGORY=happy-path E2E_TEST=
reject unknown-category E2E_CATEGORY=unknown
reject unknown-legacy E2E_TEST=unknown
reject conflicting-selectors E2E_CATEGORY=happy-path E2E_TEST=fault_recovery

rc=0
env -u E2E_CATEGORY -u E2E_TEST bash -x "$root/tests/e2e/run.sh" "$output/unset-default" >"$output/unset-default.log" 2>&1 || rc=$?
[[ "$rc" == 1 && -s "$DOCKER_SENTINEL" ]]
grep -F 'scenarios=(happy_path fault_recovery regression_catalog regression_search)' "$output/unset-default.log" >/dev/null
printf 'both unset: defaults to all four scenarios; poison Docker stopped the build\n'
