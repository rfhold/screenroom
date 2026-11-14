#!/bin/bash
set -euo pipefail
apple=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$apple/../.." && pwd)
if [ "${SCREENROOM_STATE_LOCAL_TESTS_ONLY:-0}" != 1 ] && [ "${SCREENROOM_STATE_OPERATION_TESTS_ONLY:-0}" != 1 ]; then
    : "${SCREENROOM_API_ENDPOINT:?Set SCREENROOM_API_ENDPOINT to the configured backend}"
    : "${SCREENROOM_USER_ID:?Set SCREENROOM_USER_ID to the installation local user selector}"
fi
mkdir -p "$apple/build/catalog-state"
swiftc -D DEBUG "$apple/Sources/Home/Catalog.swift" "$apple/Tests/CatalogStateTests.swift" \
    "$root/target/client-core/bindings/ScreenroomClientCore.swift" \
    -Xcc "-fmodule-map-file=$root/target/client-core/bindings/ScreenroomClientCoreFFI.modulemap" \
    -L "$root/target/client-core/apple/macos" -lscreenroom_client_core \
    -framework Security -framework SystemConfiguration -framework AppKit -o "$apple/build/catalog-state/tests"
SCREENROOM_API_PAGE_SIZE=1 SCREENROOM_E2E_SETTINGS_NAMESPACE="apple-state-$$" "$apple/build/catalog-state/tests"
