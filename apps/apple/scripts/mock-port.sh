#!/bin/bash
configure_apple_mock_port() {
    local port=${E2E_APPLE_MOCK_PORT-50061}
    case "$port" in
        ''|*[!0-9]*) echo 'E2E_APPLE_MOCK_PORT must be an integer from 1 through 65535' >&2; return 2 ;;
    esac
    if [ "${#port}" -gt 5 ] || [ "$((10#$port))" -lt 1 ] || [ "$((10#$port))" -gt 65535 ]; then
        echo 'E2E_APPLE_MOCK_PORT must be an integer from 1 through 65535' >&2
        return 2
    fi
    apple_mock_port=$((10#$port))
    apple_mock_listen="127.0.0.1:$apple_mock_port"
    apple_mock_endpoint="http://$apple_mock_listen"
}
