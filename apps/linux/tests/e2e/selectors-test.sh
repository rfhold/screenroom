#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
python3 - "$root" <<'PY'
from pathlib import Path
import shlex, sys

root = Path(sys.argv[1])
rules = (root / 'tests/e2e/Dockerfile.dockerignore').read_text().splitlines()
assert rules[0] == '**', 'Build context must remain deny-by-default'
assert '!apps/linux/**' not in rules and '!apps/**' not in rules
copies = [
    shlex.split(line)[1:]
    for line in (root / 'tests/e2e/Dockerfile').read_text().splitlines()
    if line.startswith('COPY ')
]
for source in ('apps/linux/tests/e2e/playback.rs',
               'apps/linux/scripts/check-playback-plugins.sh'):
    path = Path(source)
    assert (root.parent.parent / path).is_file(), source
    for entry in (*reversed(path.parents[:-1]), path):
        assert f'!{entry.as_posix()}' in rules, f'Missing exact allowlist entry: {entry}'
    assert any(source in copy for copy in copies), f'Missing COPY input: {source}'
print('build context: both playback COPY inputs and parents explicitly allowed; deny default preserved')
PY
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
cat >"$output/bin/bash" <<'SH'
#!/bin/bash
if [[ $1 == */scripts/backend.sh ]]; then
    printf '%s\n' "$*" >>"$BACKEND_SENTINEL"
    exit 0
fi
exec /bin/bash "$@"
SH
chmod +x "$output/bin/bash"
export PATH="$output/bin:$PATH"
export DOCKER_SENTINEL="$output/docker-invocations.log"
export BACKEND_SENTINEL="$output/backend-invocations.log"
rm -f "$DOCKER_SENTINEL" "$BACKEND_SENTINEL"
cat >"$output/descriptor.json" <<'JSON'
{"ready":true,"clientNetwork":"selector-test","clientUpstream":"selector-test:50051","instanceID":"11111111-1111-4111-8111-111111111111","publicOrigin":"http://127.0.0.1:50052","userID":"22222222-2222-4222-8222-222222222222"}
JSON
export SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json"
export SCREENROOM_BACKEND_PROJECT=screenroom-jellyfin-local
export SCREENROOM_BACKEND_ATTACH=1
export SCREENROOM_API_ENDPOINT=http://127.0.0.1:50052
export SCREENROOM_BACKEND_INSTANCE_ID=11111111-1111-4111-8111-111111111111

reject() {
    local name=$1 rc=0
    shift
    local destination="$output/$name"
    mkdir -p "$destination"
    cp "$seeds"/{home,sidebar,library,search,detail}.png "$destination/"
    (cd "$destination" && shasum ./*.png) >"$destination/before.sha"
    env -u SCREENROOM_E2E_JOURNEY -u E2E_CATEGORY -u E2E_TEST "$@" bash "$root/tests/e2e/run.sh" "$destination" >"$destination/selector.log" 2>&1 || rc=$?
    [[ "$rc" == 2 ]]
    (cd "$destination" && shasum ./*.png) >"$destination/after.sha"
    cmp "$destination/before.sha" "$destination/after.sha"
    [[ ! -e "$destination/build.log" && ! -e "$DOCKER_SENTINEL" && ! -e "$BACKEND_SENTINEL" ]]
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
reject empty-journey SCREENROOM_E2E_JOURNEY=
reject unknown-journey SCREENROOM_E2E_JOURNEY=other
reject browse-navigation SCREENROOM_E2E_PLAYBACK_PHASE=navigation
reject empty-playback-phase SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=
reject unknown-playback-phase SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=other
reject readiness-without-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=readiness
reject motion-without-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=motion
reject browse-motion SCREENROOM_E2E_PLAYBACK_PHASE=motion SCREENROOM_PLAYBACK_DIAGNOSTICS=1
reject motion-wrong-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=motion SCREENROOM_PLAYBACK_DIAGNOSTICS=true
reject idle-without-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=idle
reject browse-idle SCREENROOM_E2E_PLAYBACK_PHASE=idle SCREENROOM_PLAYBACK_DIAGNOSTICS=1
reject idle-wrong-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=idle SCREENROOM_PLAYBACK_DIAGNOSTICS=true
reject controls-without-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=controls
reject browse-controls SCREENROOM_E2E_PLAYBACK_PHASE=controls SCREENROOM_PLAYBACK_DIAGNOSTICS=1
reject controls-wrong-opt-in SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=controls SCREENROOM_PLAYBACK_DIAGNOSTICS=true
reject controls-retry SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=controls SCREENROOM_PLAYBACK_DIAGNOSTICS=1 E2E_CATEGORY=retry
reject diagnostics-during-capture SCREENROOM_E2E_JOURNEY=playback SCREENROOM_PLAYBACK_DIAGNOSTICS=1
reject unknown-diagnostics SCREENROOM_E2E_JOURNEY=playback SCREENROOM_PLAYBACK_DIAGNOSTICS=other
reject diagnostics-during-navigation SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=navigation SCREENROOM_PLAYBACK_DIAGNOSTICS=1
reject empty-diagnostics SCREENROOM_E2E_JOURNEY=playback SCREENROOM_PLAYBACK_DIAGNOSTICS=
reject playback-retry SCREENROOM_E2E_JOURNEY=playback E2E_CATEGORY=retry
reject playback-all SCREENROOM_E2E_JOURNEY=playback E2E_CATEGORY=all
reject playback-regression SCREENROOM_E2E_JOURNEY=playback E2E_CATEGORY=regression
reject playback-missing-item SCREENROOM_E2E_JOURNEY=playback E2E_CATEGORY=happy-path SCREENROOM_SHOWCASE_ITEM_ID=
reject playback-invalid-year SCREENROOM_E2E_JOURNEY=playback SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=bad SCREENROOM_BACKEND_DESCRIPTOR=descriptor
reject playback-missing-attach SCREENROOM_E2E_JOURNEY=playback SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 SCREENROOM_BACKEND_DESCRIPTOR=descriptor SCREENROOM_BACKEND_ATTACH=0
for pair in \
    'missing-project SCREENROOM_BACKEND_PROJECT=' \
    'alternate-project SCREENROOM_BACKEND_PROJECT=screenroom-local' \
    'alternate-endpoint SCREENROOM_API_ENDPOINT=http://127.0.0.1:50051' \
    'missing-endpoint SCREENROOM_API_ENDPOINT=' \
    'empty-attach SCREENROOM_BACKEND_ATTACH=' \
    'missing-instance SCREENROOM_BACKEND_INSTANCE_ID=' \
    'mismatched-instance SCREENROOM_BACKEND_INSTANCE_ID=33333333-3333-4333-8333-333333333333'; do
    read -r name assignment <<<"$pair"
    reject "playback-$name" SCREENROOM_E2E_JOURNEY=playback SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 "$assignment"
done
mkdir -p "$output/symlink-target" "$output/playback-symlink"
printf 'must remain untouched\n' >"$output/symlink-target/marker"
if [[ ! -e "$output/playback-symlink/playback" ]]; then
    ln -s "$output/symlink-target" "$output/playback-symlink/playback"
fi
reject playback-symlink SCREENROOM_E2E_JOURNEY=playback SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 SCREENROOM_BACKEND_ATTACH=1
[[ -L "$output/playback-symlink/playback" && $(cat "$output/symlink-target/marker") == 'must remain untouched' ]]

rc=0
env -u SCREENROOM_E2E_JOURNEY -u E2E_CATEGORY -u E2E_TEST -u SCREENROOM_API_ENDPOINT -u SCREENROOM_ENDPOINT -u SCREENROOM_E2E_API_ENDPOINT -u SCREENROOM_BACKEND_INSTANCE_ID SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json" bash -x "$root/tests/e2e/run.sh" "$output/unset-default" >"$output/unset-default.log" 2>&1 || rc=$?
[[ "$rc" == 1 && -s "$DOCKER_SENTINEL" ]]
grep -F 'scenarios=(happy_path fault_recovery regression_catalog regression_search)' "$output/unset-default.log" >/dev/null
printf 'both unset: defaults to all four scenarios; poison Docker stopped the build\n'

mkdir -p "$output/playback-freshness/playback"
cp "$seeds"/{home,sidebar,library,search,detail}.png "$output/playback-freshness/"
cp "$seeds/home.png" "$output/playback-freshness/playback/playing.png"
printf '{}\n' >"$output/playback-freshness/playback/manifest.json"
printf 'stale producer output\n' >"$output/playback-freshness/playback/stale-marker"
python3 - "$output/playback-freshness/playback" <<'PY'
import os,sys
os.utime(sys.argv[1], (946684800, 946684800))
PY
(cd "$output/playback-freshness" && shasum ./*.png) >"$output/playback-before.sha"
rc=0
env -u E2E_CATEGORY -u E2E_TEST -u SCREENROOM_ENDPOINT -u SCREENROOM_E2E_API_ENDPOINT \
    SCREENROOM_E2E_JOURNEY=playback SCREENROOM_BACKEND_ATTACH=1 SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json" \
    SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 \
    bash -x "$root/tests/e2e/run.sh" "$output/playback-freshness" >"$output/playback-freshness.log" 2>&1 || rc=$?
[[ "$rc" == 1 && -d "$output/playback-freshness/playback" && ! -e "$output/playback-freshness/playback/manifest.json" && ! -e "$output/playback-freshness/playback/playing.png" && ! -e "$output/playback-freshness/playback/stale-marker" ]]
python3 - "$output/playback-freshness/playback" <<'PY'
import os,sys,time
assert os.stat(sys.argv[1]).st_mtime > time.time() - 60
PY
grep -F 'scenarios=(playback_showcase)' "$output/playback-freshness.log" >/dev/null
(cd "$output/playback-freshness" && shasum ./*.png) >"$output/playback-after.sha"
cmp "$output/playback-before.sha" "$output/playback-after.sha"
printf 'playback default: exact playback_showcase; owned directory recreated fresh; failed build clears success evidence; browse hashes unchanged\n'

rc=0
env -u E2E_CATEGORY -u E2E_TEST -u SCREENROOM_ENDPOINT -u SCREENROOM_E2E_API_ENDPOINT \
    SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=navigation \
    SCREENROOM_BACKEND_ATTACH=1 SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json" \
    SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 \
    bash -x "$root/tests/e2e/run.sh" "$output/navigation" >"$output/navigation.log" 2>&1 || rc=$?
[[ "$rc" == 1 && ! -e "$output/navigation/playback/manifest.json" ]]
grep -F 'scenarios=(playback_showcase)' "$output/navigation.log" >/dev/null
grep -F 'phase=navigation' "$output/navigation.log" >/dev/null
printf 'navigation diagnostic: same playback test; explicit phase; poison Docker stopped build; no manifest\n'

rc=0
env -u E2E_CATEGORY -u E2E_TEST -u SCREENROOM_ENDPOINT -u SCREENROOM_E2E_API_ENDPOINT \
    SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=readiness SCREENROOM_PLAYBACK_DIAGNOSTICS=1 \
    SCREENROOM_BACKEND_ATTACH=1 SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json" \
    SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 \
    bash -x "$root/tests/e2e/run.sh" "$output/readiness" >"$output/readiness.log" 2>&1 || rc=$?
[[ "$rc" == 1 && ! -e "$output/readiness/playback/manifest.json" ]]
grep -F 'phase=readiness' "$output/readiness.log" >/dev/null
grep -F 'diagnostics=1' "$output/readiness.log" >/dev/null
printf 'readiness diagnostic: explicit opt-in; same playback test; poison Docker stopped build; no manifest\n'

mkdir -p "$output/motion"
printf 'stale\n' >"$output/motion/playback-motion.json"
cp "$seeds/home.png" "$output/motion/playback-motion-first.png"
cp "$seeds/home.png" "$output/motion/playback-motion-second.png"
rc=0
env -u E2E_CATEGORY -u E2E_TEST -u SCREENROOM_ENDPOINT -u SCREENROOM_E2E_API_ENDPOINT \
    SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=motion SCREENROOM_PLAYBACK_DIAGNOSTICS=1 \
    SCREENROOM_BACKEND_ATTACH=1 SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json" \
    SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 \
    bash -x "$root/tests/e2e/run.sh" "$output/motion" >"$output/motion.log" 2>&1 || rc=$?
[[ "$rc" == 1 && ! -e "$output/motion/playback/manifest.json" ]]
[[ ! -e "$output/motion/playback-motion.json" && ! -e "$output/motion/playback-motion-first.png" && ! -e "$output/motion/playback-motion-second.png" ]]
grep -F 'phase=motion' "$output/motion.log" >/dev/null
grep -F 'diagnostics=1' "$output/motion.log" >/dev/null
printf 'motion diagnostic: explicit opt-in; stale pair/sidecar cleared; no manifest; poison Docker stopped build\n'

mkdir -p "$output/idle"
for suffix in .json .png -tree.xml -decisions.jsonl; do
    printf 'stale\n' >"$output/idle/playback-idle$suffix"
done
rc=0
env -u E2E_CATEGORY -u E2E_TEST -u SCREENROOM_ENDPOINT -u SCREENROOM_E2E_API_ENDPOINT \
    SCREENROOM_E2E_JOURNEY=playback SCREENROOM_E2E_PLAYBACK_PHASE=idle SCREENROOM_PLAYBACK_DIAGNOSTICS=1 \
    SCREENROOM_BACKEND_ATTACH=1 SCREENROOM_BACKEND_DESCRIPTOR="$output/descriptor.json" \
    SCREENROOM_SHOWCASE_ITEM_ID=exact SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 \
    bash -x "$root/tests/e2e/run.sh" "$output/idle" >"$output/idle.log" 2>&1 || rc=$?
[[ "$rc" == 1 && ! -e "$output/idle/playback/manifest.json" ]]
for suffix in .json .png -tree.xml -decisions.jsonl; do
    [[ ! -e "$output/idle/playback-idle$suffix" ]]
done
grep -F 'phase=idle' "$output/idle.log" >/dev/null
grep -F 'diagnostics=1' "$output/idle.log" >/dev/null
printf 'idle diagnostic: explicit opt-in; stale local diagnostics cleared; no manifest; poison Docker stopped build\n'
