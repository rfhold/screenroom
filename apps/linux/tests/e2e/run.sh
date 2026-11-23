#!/usr/bin/env bash
set -euo pipefail

journey=${SCREENROOM_E2E_JOURNEY-browse}
case "$journey" in
    browse|playback) ;;
    *) printf 'Unknown SCREENROOM_E2E_JOURNEY: %s\n' "$journey" >&2; exit 2 ;;
esac
phase=${SCREENROOM_E2E_PLAYBACK_PHASE-capture}
case "$journey:$phase" in
    browse:capture|playback:capture|playback:navigation|playback:readiness|playback:motion|playback:idle|playback:controls) ;;
    *) printf 'Invalid SCREENROOM_E2E_PLAYBACK_PHASE for journey\n' >&2; exit 2 ;;
esac
diagnostics=${SCREENROOM_PLAYBACK_DIAGNOSTICS-0}
case "$diagnostics:$phase" in
    0:capture|0:navigation|1:readiness|1:motion|1:idle|1:controls) ;;
    *) printf 'Readiness, motion, idle and controls require SCREENROOM_PLAYBACK_DIAGNOSTICS=1; diagnostics are phase-specific\n' >&2; exit 2 ;;
esac
category=${E2E_CATEGORY-all}
if [[ "$journey" == playback && ${E2E_CATEGORY+x} != x ]]; then category=happy-path; fi
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
if [[ "$journey" == playback ]]; then
    if [[ "$category" != happy-path ]]; then
        printf 'Playback requires E2E_CATEGORY=happy-path\n' >&2; exit 2
    fi
    for selector in SCREENROOM_SHOWCASE_ITEM_ID SCREENROOM_SHOWCASE_TITLE SCREENROOM_SHOWCASE_YEAR SCREENROOM_BACKEND_DESCRIPTOR; do
        if [[ -z ${!selector:-} ]]; then printf '%s is required for playback\n' "$selector" >&2; exit 2; fi
    done
    if [[ ! $SCREENROOM_SHOWCASE_YEAR =~ ^[1-9][0-9]{3}$ ]]; then printf 'Invalid showcase year\n' >&2; exit 2; fi
    if [[ ${SCREENROOM_BACKEND_ATTACH:-} != 1 ]]; then printf 'Playback requires explicit backend attach\n' >&2; exit 2; fi
    if [[ ${SCREENROOM_BACKEND_PROJECT:-} != screenroom-jellyfin-local ]]; then printf 'Playback requires SCREENROOM_BACKEND_PROJECT=screenroom-jellyfin-local\n' >&2; exit 2; fi
    if [[ ${SCREENROOM_API_ENDPOINT:-} != http://127.0.0.1:50052 ]]; then printf 'Playback requires SCREENROOM_API_ENDPOINT=http://127.0.0.1:50052\n' >&2; exit 2; fi
    if [[ -z ${SCREENROOM_BACKEND_INSTANCE_ID:-} ]]; then printf 'Playback requires explicit SCREENROOM_BACKEND_INSTANCE_ID\n' >&2; exit 2; fi
    python3 - "$SCREENROOM_BACKEND_DESCRIPTOR" "$SCREENROOM_BACKEND_INSTANCE_ID" "$SCREENROOM_API_ENDPOINT" <<'PY' || exit 2
import json,sys
with open(sys.argv[1]) as source:
    descriptor = json.load(source)
if descriptor.get('instanceID') != sys.argv[2] or descriptor.get('publicOrigin') != sys.argv[3]:
    raise SystemExit('Playback descriptor conflicts with the explicit caller instance/origin')
PY
    scenarios=(playback_showcase)
fi

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
output=${1:-"$root/build/e2e"}
mkdir -p "$output"
output=$(cd -- "$output" && pwd)
playback_output="$output/playback"
if [[ "$journey" == playback && ( -L "$playback_output" || ( -e "$playback_output" && ! -d "$playback_output" ) ) ]]; then
    printf 'Playback output must be a producer-owned directory, not a symlink or file\n' >&2; exit 2
fi
container=
screenshots=("$output"/{home,sidebar,library,search,detail}.png)
screenshots+=("$output/happy_path"/{home,sidebar,library,search,detail}.png)
if [[ "$journey" == playback ]]; then
    screenshots=("$output/playback/manifest.json" "$output/playback/"{detail,loading,playing,paused,idle,settings,volume-min,volume-max,muted,seek-forward,seek-back,tracks,subtitles-off,fullscreen,media-error,stopped,cleanup}.png)
fi
cleanup() {
    local status=$?
    if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null || true; fi
    if [[ "$status" != 0 ]]; then
        rm -f "${screenshots[@]}"
        if [[ "$journey" == playback ]]; then
            rm -f "$playback_output"/{audio,subtitle}-track-*.png
        fi
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
# A failed rerun must not retain earlier success captures from this journey.
if [[ "$journey" == playback ]]; then
    # output is canonical; this exact nonsymlink child belongs only to this producer.
    rm -rf -- "$playback_output"
    mkdir -- "$playback_output"
    rm -f "$output"/playback-{navigation.png,readiness.png,resource-baseline.json,cleanup-context.json,owned.pid,stages.log,failure.png,failure-tree.xml,failure-context.json}
    rm -f "$output"/playback-motion{.json,-first.png,-second.png}
    rm -f "$output"/playback-idle{.json,.png,-tree.xml,-decisions.jsonl}
    rm -f "$output/playback-controls.jsonl"
fi
rm -f "${screenshots[@]}"
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
namespace=${SCREENROOM_E2E_SETTINGS_NAMESPACE:-${E2E_PROFILE_PREFIX:-linux-$(date +%s)-$$}}
if [[ ! $namespace =~ ^[a-zA-Z0-9_-]+$ ]]; then printf 'Invalid local settings namespace\n' >&2; exit 2; fi
DOCKER_BUILDKIT=1 docker build --progress=plain -f "$root/tests/e2e/Dockerfile" -t "$image" "$root/../.." >"$output/build.log" 2>&1 || {
    cat "$output/build.log"
    exit 1
}
: >"$output/e2e.log"
status=0
pids_limit=128
if [[ "$journey" == playback ]]; then pids_limit=512; fi
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
        -e SCREENROOM_E2E_JOURNEY="$journey" \
        -e SCREENROOM_E2E_PLAYBACK_PHASE="$phase" \
        -e SCREENROOM_PLAYBACK_DIAGNOSTICS="$diagnostics" \
        -e SCREENROOM_SHOWCASE_ITEM_ID -e SCREENROOM_SHOWCASE_TITLE -e SCREENROOM_SHOWCASE_YEAR \
        -e E2E_REQUIRED_LABEL -e E2E_RESOLUTION -e E2E_APP -e E2E_SCENARIO="$scenario" \
        --pids-limit "$pids_limit" --memory 2g "$image")
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
if [[ "$journey" == playback && "$status" == 0 ]]; then
    if [[ "$phase" == motion ]]; then
        if ! python3 - "$output" <<'PY'
import json, math, pathlib, struct, sys
root = pathlib.Path(sys.argv[1])
assert not any((root / 'playback').iterdir()), 'Motion diagnostics must not export playback evidence'
data = json.loads((root / 'playback-motion.json').read_text())
assert data['schemaVersion'] == 1 and data['check'] == 'motion' and data['status'] == 'passed'
assert data['viewport'] == [1280, 800]
assert data['region'] == {'x': 420, 'y': 260, 'width': 440, 'height': 280}
assert data['total_pixels'] == 123200 and data['channel_delta_strictly_greater_than'] == 12
assert data['required_changed_pixels'] == 1000 and data['requested_sleep_ms'] == 2000
assert type(data['changed_pixels']) is int and 1000 <= data['changed_pixels'] <= 123200
samples = data['samples']
assert len(samples) == 2
for sample, name in zip(samples, ['playback-motion-first.png', 'playback-motion-second.png']):
    assert sample['file'] == name and sample['retained'] is True
    assert sample['capture_status'] == 'acquired' and sample['validation'] == 'passed'
    assert sample['dimensions'] == [1280, 800]
    start, end = sample['capture_start_ms'], sample['capture_end_ms']
    assert type(start) is int and type(end) is int and 0 <= start <= end
    png = (root / name).read_bytes()
    assert png[:8] == b'\x89PNG\r\n\x1a\n' and struct.unpack('>II', png[16:24]) == (1280, 800)
assert samples[1]['capture_start_ms'] - samples[0]['capture_end_ms'] >= 2000
for slot in ['before', 'after']:
    observation = data['timeline'][slot]
    start, end = observation['elapsed_start_ms'], observation['elapsed_end_ms']
    assert type(start) is int and type(end) is int and 0 <= start <= end
    value = observation['position_ms']
    assert ((observation['status'] == 'unavailable' and value is None) or
            (observation['status'] == 'observed' and type(value) in (int, float) and math.isfinite(value) and value >= 0))
assert data['timeline']['before']['elapsed_end_ms'] <= samples[0]['capture_start_ms']
assert samples[1]['capture_end_ms'] <= data['timeline']['after']['elapsed_start_ms']
assert data['timeline']['same_slider'] is None or type(data['timeline']['same_slider']) is bool
PY
        then
            printf 'Motion diagnostic requires the exact fresh comparison pair, safe observations and no export\n' >&2
            status=1
        fi
        if ! grep -F 'playback-stage: motion-comparison end' "$output/e2e.log" >/dev/null \
            || grep -F 'playback-stage: captures-controls begin' "$output/e2e.log" >/dev/null; then
            printf 'Motion diagnostic must execute only its comparison before normal cleanup\n' >&2
            status=1
        fi
    elif [[ "$phase" == controls ]]; then
        if ! python3 "$root/tests/e2e/controls-runner-test.py" --validate "$output"; then
            printf 'Controls diagnostic requires complete prefix provenance and no export\n' >&2
            status=1
        fi
        if ! grep -F 'playback-stage: controls-prefix end' "$output/e2e.log" >/dev/null \
            || grep -E 'playback-stage: (motion-comparison|idle-observation|captures-controls) begin' "$output/e2e.log" >/dev/null; then
            printf 'Controls diagnostic must execute only the shared prefix before cleanup\n' >&2
            status=1
        fi
    elif [[ "$phase" == idle ]]; then
        if ! python3 "$root/tests/e2e/idle-evidence-test.py" --validate "$output"
        then
            printf 'Idle diagnostic requires safe decision records, fresh image and no export\n' >&2
            status=1
        fi
        if ! grep -F 'playback-stage: idle-observation end' "$output/e2e.log" >/dev/null \
            || grep -E 'playback-stage: (motion-comparison|captures-controls) begin' "$output/e2e.log" >/dev/null; then
            printf 'Idle diagnostic must observe only idle before normal cleanup\n' >&2
            status=1
        fi
    elif [[ "$phase" == readiness ]]; then
        if [[ ! -s "$output/playback-readiness.png" || -e "$output/playback/manifest.json" ]]; then
            printf 'Readiness diagnostic requires fresh evidence and no manifest\n' >&2
            status=1
        fi
    elif [[ "$phase" == navigation ]]; then
        if [[ ! -s "$output/playback-navigation.png" || -e "$output/playback/manifest.json" ]] \
            || ! grep -F 'playback-stage: detail end' "$output/e2e.log" >/dev/null \
            || grep -F 'playback-stage: Play begin' "$output/e2e.log" >/dev/null; then
            printf 'Navigation requires fresh details evidence, no Play and no manifest\n' >&2
            status=1
        fi
    elif [[ ! -s "$output/playback/manifest.json" ]]; then
        printf 'Playback requires a fresh manifest\n' >&2
        status=1
    fi
    if ! grep -F 'test result: ok. 1 passed; 0 failed; 0 ignored;' "$output/e2e.log" >/dev/null; then
        printf 'Playback requires a fresh manifest and one executed, non-skipped test\n' >&2
        status=1
    fi
fi
rm -f "$output/current-scenario.log"
if [[ "$status" != 0 ]]; then rm -f "${screenshots[@]}"; fi
cat "$output/e2e.log"
printf 'Artifacts: %s\n' "$output"
exit "$status"
