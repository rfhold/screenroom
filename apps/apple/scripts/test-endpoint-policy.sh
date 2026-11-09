#!/bin/bash
set -euo pipefail
apple=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$apple/../.." && pwd)
bash "$apple/scripts/prepare-core.sh"
out="$apple/build/endpoint-policy"
mkdir -p "$out"
for mode in debug release; do
    flags=()
    if [ "$mode" = debug ]; then flags+=(-D DEBUG); fi
    swiftc "${flags[@]}" "$apple/Sources/Home/Catalog.swift" "$apple/Tests/EndpointPolicyTests.swift" \
        "$root/target/client-core/bindings/ScreenroomClientCore.swift" \
        -Xcc "-fmodule-map-file=$root/target/client-core/bindings/ScreenroomClientCoreFFI.modulemap" \
        -L "$root/target/client-core/apple/macos" -lscreenroom_client_core \
        -framework Security -framework SystemConfiguration -o "$out/$mode"
    for endpoint in https://example.invalid http://127.0.0.1:50051 http://localhost:50051 'http://[::1]:50051' http://192.0.2.1:50051 ftp://localhost 'https://user:password@example.invalid' invalid; do
        allowed=0
        if [ "$endpoint" = https://example.invalid ]; then allowed=1; fi
        if [ "$mode" = debug ]; then
            case "$endpoint" in http://127.0.0.1:*|http://localhost:*|'http://[::1]:'*) allowed=1;; esac
        fi
        SCREENROOM_API_ENDPOINT="$endpoint" EXPECT_ENDPOINT_ALLOWED="$allowed" "$out/$mode"
    done
done
