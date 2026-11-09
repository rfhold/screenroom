#!/bin/bash
set -euo pipefail
source "$(dirname "$0")/mock-port.sh"
unset E2E_APPLE_MOCK_PORT
configure_apple_mock_port
test "$apple_mock_endpoint" = http://127.0.0.1:50061
test "$apple_mock_listen" = 127.0.0.1:50061
for port in 1 65535 54321; do
    E2E_APPLE_MOCK_PORT=$port configure_apple_mock_port
    test "$apple_mock_endpoint" = "http://127.0.0.1:$port"
    test "$apple_mock_listen" = "127.0.0.1:$port"
done
for port in '' 0 65536 -1 abc 12.3 12345678901234567890; do
    if E2E_APPLE_MOCK_PORT=$port configure_apple_mock_port 2>/dev/null; then
        echo "Invalid port accepted: $port" >&2
        exit 1
    fi
done
echo 'Apple mock port tests passed'
