#!/usr/bin/env bash
set -euo pipefail
server=
cleanup() {
    if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
scenario=${E2E_SCENARIO:-happy_path}
case "$scenario" in
    happy_path|regression_catalog|regression_search) ;;
    fault_recovery)
        export E2E_APP=${E2E_APP:-screenroom-linux-capture}
        dbus-run-session -- /usr/local/bin/e2e-test fault_recovery --exact --nocapture --test-threads=1
        exit "$?"
        ;;
    *) printf 'Unknown E2E_SCENARIO: %s\n' "$scenario" >&2; exit 2 ;;
esac
screenroom-mock-server > /artifacts/mock-server.log 2>&1 &
server=$!
for attempt in {1..100}; do
    if curl --fail --silent http://127.0.0.1:50051/health > /artifacts/mock-health.txt; then break; fi
    kill -0 "$server"
    sleep 0.02
done
curl --fail --silent http://127.0.0.1:50051/health
export E2E_APP=${E2E_APP:-screenroom-linux-capture}
dbus-run-session -- /usr/local/bin/e2e-test "$scenario" --exact --nocapture --test-threads=1
