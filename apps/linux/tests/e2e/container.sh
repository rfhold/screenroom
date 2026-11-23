#!/usr/bin/env bash
set -euo pipefail
scenario=${E2E_SCENARIO:-happy_path}
case "${SCREENROOM_E2E_JOURNEY:-browse}:$scenario" in
    browse:happy_path|browse:regression_catalog|browse:regression_search|browse:fault_recovery|playback:playback_showcase) ;;
    *) printf 'Invalid journey/scenario selection\n' >&2; exit 2 ;;
esac
server=
cleanup() {
    if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
test -n "${SCREENROOM_BACKEND_INSTANCE_ID:?shared backend instance required}"
python3 /usr/local/bin/screenroom-origin.py --forward &
server=$!
for attempt in {1..50}; do
    if curl --fail --silent --connect-timeout 1 --max-time 2 "$SCREENROOM_API_ENDPOINT/__screenroom_instance" > /artifacts/backend-instance.json; then break; fi
    sleep 0.1
done
python3 -c 'import json,os; d=json.load(open("/artifacts/backend-instance.json")); assert d["instanceID"] == os.environ["SCREENROOM_BACKEND_INSTANCE_ID"], d'
export SCREENROOM_SETTINGS_PATH="/artifacts/settings-${SCREENROOM_E2E_SETTINGS_NAMESPACE:?}.json"
printf '%s\n' "$SCREENROOM_SETTINGS_PATH" > /artifacts/settings-path.txt
case "$scenario" in
    happy_path|regression_catalog|regression_search|playback_showcase) ;;
    fault_recovery)
        kill "$server"; wait "$server" || true; server=
        export E2E_APP=${E2E_APP:-screenroom-linux-capture}
        dbus-run-session -- /usr/local/bin/e2e-test fault_recovery --exact --nocapture --test-threads=1
        exit "$?"
        ;;
    *) printf 'Unknown E2E_SCENARIO: %s\n' "$scenario" >&2; exit 2 ;;
esac
export E2E_APP=${E2E_APP:-screenroom-linux-capture}
dbus-run-session -- /usr/local/bin/e2e-test "$scenario" --exact --nocapture --test-threads=1
