#!/bin/bash
set -euo pipefail
apple=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$apple/../.." && pwd)
: "${SCREENROOM_API_ENDPOINT:?Set SCREENROOM_API_ENDPOINT to the shared backend}"
mkdir -p "$apple/build/catalog-state"
swiftc -D DEBUG "$apple/Sources/Home/Catalog.swift" "$apple/Tests/CatalogStateTests.swift" \
    "$root/target/client-core/bindings/ScreenroomClientCore.swift" \
    -Xcc "-fmodule-map-file=$root/target/client-core/bindings/ScreenroomClientCoreFFI.modulemap" \
    -L "$root/target/client-core/apple/macos" -lscreenroom_client_core \
    -framework Security -framework SystemConfiguration -framework AppKit -o "$apple/build/catalog-state/tests"
SCREENROOM_API_PAGE_SIZE=1 SCREENROOM_E2E_SETTINGS_NAMESPACE="apple-state-$$" "$apple/build/catalog-state/tests"
