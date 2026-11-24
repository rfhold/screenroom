#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p build/backend-runtime
runtime="$PWD/build/backend-runtime/screenroom-shell-test-$(openssl rand -hex 12)"
(umask 077; mkdir "$runtime")
trap 'rm -r "$runtime"' EXIT
export SCREENROOM_BACKEND_PROJECT=${runtime##*/}
export SCREENROOM_BACKEND_PORT=49611
export SCREENROOM_API_ENDPOINT=http://127.0.0.1:49611
export SCREENROOM_E2E_API_ENDPOINT=$SCREENROOM_API_ENDPOINT
unset SCREENROOM_BACKEND_PROFILE
export TEST_RUNTIME="$runtime"
export PATH="$runtime/bin:$PATH"
mkdir "$runtime/bin"

cat > "$runtime/bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$TEST_RUNTIME/calls"
if [[ ${1-} == volume ]]; then
  if [[ ${TEST_EXISTING_VOLUMES-} == yes ]]; then printf '%s\n' fixture-volume; fi
  exit 0
fi
if [[ ${1-} == ps ]]; then
  if [[ ${TEST_RUNNING-} == yes ]]; then printf '%s\n' fixture-container; fi
  exit 0
fi
if [[ ${1-} == stop ]]; then exit 0; fi
shift 7
case "$*" in
  'ps --all --services') printf 'catalog\nworker\nplayback-gateway\n' ;;
  'ps --status running --services') printf 'catalog\nplayback-gateway\n' ;;
  'exec -T playback-gateway curl -fsS --max-time 3 http://127.0.0.1:50054/ready')
    [[ ${TEST_PLAYBACK_DOWN-} != yes ]] ;;
  'exec -T garage /garage status'|'exec -T garage /garage layout show'|'exec -T garage /garage bucket '*|'exec -T garage /garage key '*) ;;
  'up -d --wait postgres garage'|'up -d user catalog worker playback-gateway index-operator edge'|'down --timeout 75') ;;
  'run --rm --no-deps --user 0:0 --entrypoint /bin/sh '*) ;;
  'exec -T postgres psql -XAt '*) cat >"$TEST_RUNTIME/operator-sql"; printf 'not descriptor output\n' ;;
  'exec -T postgres psql '*) ;;
  'run --rm --no-deps --name '*' worker screenroom-index-worker drain')
    if [[ ${TEST_BLOCK_DRAIN-} == yes ]]; then
      printf '%s\n' "$$" >"$TEST_RUNTIME/blocked-drain"
      exec sleep 60
    fi
    exit 9 ;;
  'run --rm -T --no-deps user screenroom-user-init --create-missing -')
    cat >"$TEST_RUNTIME/user-input"
    if [[ -n ${SCREENROOM_TEST_USER_INIT-} ]]; then
      env -u DATABASE_URL -u SCREENROOM_MIGRATION_DATABASE_URL "$SCREENROOM_TEST_USER_INIT" --create-missing - \
        <"$TEST_RUNTIME/user-input" >"$TEST_RUNTIME/initializer-output" 2>&1 && exit 7
      grep -q 'missing environment configuration' "$TEST_RUNTIME/initializer-output"
    fi ;;
  *) exit 53 ;;
esac
SH
cat > "$runtime/bin/curl" <<'SH'
#!/usr/bin/env bash
if [[ $* == *--write-out* ]]; then
  [[ $(wc -c | tr -d '[:space:]') == 5 ]] || exit 53
  printf '%s' "${TEST_PRIVATE_CODE:-404}"
else cat "$TEST_RUNTIME/identity.json"; fi
SH
cat > "$runtime/bin/grpcurl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "${@: -1}" in
  */GetCatalogInfo)
    if [[ ${TEST_LARGE_RPC-} == yes ]]; then cat "$TEST_RUNTIME/large-rpc"; else printf '{}\n'; fi ;;
  */GetHome|*/ListLibraries|*/GetSearchCapabilities|*/Health) printf '{}\n' ;;
  */GetCurrentUser) jq -nc --arg user "$(jq -er '.userID' "$TEST_RUNTIME/local-user.json")" '{user:{id:$user}}' ;;
  */ObserveQueue) jq -nc '{payloadJson: ({queue:{ready:2,blocked:null,hidden:"private"},other:"private"}|tojson|@base64)}' ;;
  *) exit 53 ;;
esac
SH
cat > "$runtime/bin/buf" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' 'buf: } (#1) [5 bytes data]' 'buf: < (#1) HTTP/1.1 200 OK' \
  "buf: < (#1) Access-Control-Allow-Origin: ${TEST_CORS:-http://127.0.0.1:4173}" \
  "buf: { (#1) [${TEST_WIRE_BYTES:-25} bytes data]" >&2
case "${@: -1}" in
  */GetCurrentUser) jq -nc --arg user "$(jq -er '.userID' "$TEST_RUNTIME/local-user.json")" '{user:{id:$user}}' ;;
  */GetCatalogInfo)
    if [[ ${TEST_LARGE_RPC-} == yes ]]; then cat "$TEST_RUNTIME/large-rpc"; else printf '{}\n'; fi ;;
  *) printf '{}\n' ;;
esac
exit "${TEST_GRPC_STATUS:-0}"
SH
chmod +x "$runtime/bin/"*

printf '%s\n' 'another-project' > "$runtime/wrong owner"
if bash scripts/backend-persistence.sh "$runtime/wrong owner" 2> "$runtime/error"; then
  exit 1
fi
grep -q 'teardown owner mismatch' "$runtime/error"
[[ ! -e $runtime/calls ]]

cat > "$runtime/identity.json" <<'JSON'
{"instanceID":"fixture","publicOrigin":"http://127.0.0.1:49611"}
JSON
jq -c '. + {ready:true,userID:"fixture-user"}' "$runtime/identity.json" > "$runtime/descriptor.json"
status=$(bash scripts/backend.sh status --live)
jq -e '.attachment == "unknown" and .current.worker == {process:"stopped",loop:"unknown",degraded:true}' \
  >/dev/null <<< "$status"
[[ ! -e $runtime/local-user.json && ! -e $runtime/runtime.env ]]
printf '%s\n' '{"userID":"fixture-user"}' > "$runtime/local-user.json"
before=$(shasum -a 256 "$runtime/"*.json)
status=$(bash scripts/backend.sh status --live)
jq -e '.attachment == "last_verified" and .current.edge == "matched" and .current.catalog == "ready"
       and .current.playback == "ready" and .current.queue == {ready:2,blocked:null}' >/dev/null <<< "$status"
[[ $(shasum -a 256 "$runtime/"*.json) == "$before" ]]
[[ $(printf 'select 1;\n' | python3 scripts/backend-process.py --stdin 5 32 cat) == 'select 1;' ]]
if printf '%064d\n' 0 | python3 scripts/backend-process.py --stdin 5 32 cat \
    > "$runtime/bounded-out" 2> "$runtime/bounded-error"; then
  exit 1
fi
[[ ! -s $runtime/bounded-out ]]
grep -q 'execution budget' "$runtime/bounded-error"
python3 - <<'PY'
import os, runpy, signal, subprocess, sys
from unittest.mock import patch

real_popen = subprocess.Popen
started = []

def interrupt_during_spawn(*args, **kwargs):
    child = real_popen(*args, **kwargs)
    started.append(child)
    os.kill(os.getpid(), signal.SIGTERM)
    return child

with patch.object(sys, 'argv', ['backend-process.py', '10', '1024', 'sleep', '60']):
    with patch.object(subprocess, 'Popen', interrupt_during_spawn):
        try:
            runpy.run_path('scripts/backend-process.py', run_name='__main__')
        except SystemExit as error:
            assert error.code == 143, error
        else:
            raise AssertionError('expected SIGTERM exit')
assert len(started) == 1 and started[0].returncode == -signal.SIGKILL
PY

rm "$runtime/"*.json
owner="$runtime/explicit owner"
SCREENROOM_BACKEND_BUILD=0 bash scripts/backend.sh up --owner-file "$owner" >"$runtime/wait-output"
[[ $(cat "$runtime/wait-output") == "$runtime/descriptor.json" && $(cat "$owner") == "$SCREENROOM_BACKEND_PROJECT" ]]
grep -q 'CREATE ROLE operator_runtime' "$runtime/operator-sql"
jq -e 'length == 1 and (.[0] | keys == ["display_name","id"])' "$runtime/user-input" >/dev/null
user=$(jq -er '.[0].id' "$runtime/users.json")
bash scripts/backend.sh status | jq -e --arg user "$user" '.ready and .userID == $user' >/dev/null
bash scripts/backend-contract.sh "$SCREENROOM_API_ENDPOINT" "$user" |
  jq -e '.nativeGrpc and .grpcWeb and .privateRoutesAbsent' >/dev/null
jq -nc '{value:("x"*160000)}' >"$runtime/large-rpc"
TEST_LARGE_RPC=yes TEST_WIRE_BYTES=160100 bash scripts/backend-contract.sh "$SCREENROOM_API_ENDPOINT" "$user" |
  jq -e '.nativeGrpc and .grpcWeb' >/dev/null
if TEST_PRIVATE_CODE=200 bash scripts/backend-contract.sh "$SCREENROOM_API_ENDPOINT" "$user" >/dev/null 2>&1; then exit 1; fi
jq '.[0].links=[]' "$runtime/users.json" >"$runtime/legacy"
mv "$runtime/legacy" "$runtime/users.json"
before=$(shasum -a 256 "$runtime/"*.json)
SCREENROOM_BACKEND_BUILD=0 bash scripts/backend.sh up --owner-file "$owner" >"$runtime/wait-output"
[[ $(shasum -a 256 "$runtime/"*.json) == "$before" ]]
jq -e '.[0] | has("links") | not' "$runtime/user-input" >/dev/null
bash scripts/backend.sh down --owner-file "$owner"
[[ ! -e $runtime/descriptor.json && -e $runtime/identity.json && -e $runtime/runtime.env ]]
for fault in TEST_CORS=http://wrong.example TEST_WIRE_BYTES=2097153 TEST_GRPC_STATUS=24 TEST_PLAYBACK_DOWN=yes; do
  if env "$fault" SCREENROOM_BACKEND_WAIT_SECONDS=2 bash scripts/backend.sh wait >/dev/null 2>&1; then exit 1; fi
  [[ ! -e $runtime/descriptor.json ]]
done
if bash scripts/backend.sh drain >/dev/null 2>&1; then exit 1; fi
operation=$(sed -n 's/.*--name \([^ ]*\) worker screenroom-index-worker drain/\1/p' "$runtime/calls" | tail -1)
[[ $operation == "$SCREENROOM_BACKEND_PROJECT-operation-"* ]]
grep -Fx "stop --time 15 $operation" "$runtime/calls" >/dev/null
TEST_BLOCK_DRAIN=yes python3 - "$runtime" <<'PY'
import os, pathlib, re, signal, subprocess, sys, time
runtime = pathlib.Path(sys.argv[1])
with subprocess.Popen(['bash', 'scripts/backend.sh', 'drain'],
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as driver:
    try:
        child_file = runtime / 'blocked-drain'
        deadline = time.monotonic() + 5
        while not child_file.exists() or not child_file.read_text().strip():
            assert time.monotonic() < deadline, 'worker stub did not start'
            time.sleep(0.02)
        child = int(child_file.read_text())
        driver.send_signal(signal.SIGTERM)
        assert driver.wait(timeout=5) == 143
        calls = (runtime / 'calls').read_text()
        operation = re.findall(r'--name (\S+) worker screenroom-index-worker drain', calls)[-1]
        assert f'stop --time 15 {operation}\n' in calls
        try:
            os.kill(child, 0)
        except ProcessLookupError:
            pass
        else:
            raise AssertionError('worker process survived driver termination')
    finally:
        if driver.poll() is None:
            driver.kill()
PY
if TEST_RUNNING=yes bash scripts/backend.sh up --owner-file "$owner" >/dev/null 2>&1; then exit 1; fi
if bash scripts/backend.sh down --owner-file "$runtime/wrong-owner" >/dev/null 2>&1; then exit 1; fi
for entry in runtime.env identity.json users.json; do
  mv "$runtime/$entry" "$runtime/saved"
  if TEST_EXISTING_VOLUMES=yes bash scripts/backend.sh initialize >/dev/null 2>&1; then exit 1; fi
  [[ ! -e $runtime/$entry ]]
  mv "$runtime/saved" "$runtime/$entry"
done
if SCREENROOM_EMBEDDING_MODEL=partial bash scripts/backend.sh initialize >/dev/null 2>&1; then exit 1; fi
before=$(shasum -a 256 "$runtime/runtime.env" "$runtime/identity.json")
jq '.[0].links=[{}]' "$runtime/users.json" >"$runtime/legacy"
mv "$runtime/legacy" "$runtime/users.json"
if bash scripts/backend.sh up --owner-file "$runtime/new owner" >/dev/null 2>"$runtime/error"; then exit 1; fi
grep -q 'unsupported links' "$runtime/error"
[[ ! -e $runtime/new\ owner && $(shasum -a 256 "$runtime/runtime.env" "$runtime/identity.json") == "$before" ]]
python3 - "$runtime/lifecycle.lock" <<'PY'
import fcntl, subprocess, sys
with open(sys.argv[1], 'a') as file:
    fcntl.flock(file, fcntl.LOCK_EX)
    result = subprocess.run(['bash', 'scripts/backend.sh', 'initialize'], capture_output=True)
    assert result.returncode and b'lock is busy' in result.stderr, result
PY
printf '%s\n' 'Backend shell checks passed.'
