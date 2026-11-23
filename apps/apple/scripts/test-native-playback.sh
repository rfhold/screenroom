#!/bin/bash
set -euo pipefail
apple=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$apple/../.." && pwd)
mkdir -p "$apple/build/native-playback"
swiftc -swift-version 6 "$apple/Sources/Home/NativePlayback.swift" "$apple/Tests/NativePlaybackTests.swift" \
    "$root/target/client-core/bindings/ScreenroomClientCore.swift" \
    -Xcc "-fmodule-map-file=$root/target/client-core/bindings/ScreenroomClientCoreFFI.modulemap" \
    -L "$root/target/client-core/apple/macos" -lscreenroom_client_core \
    -framework Security -framework SystemConfiguration -framework AVKit -framework SwiftUI \
    -o "$apple/build/native-playback/tests"
"$apple/build/native-playback/tests"
