#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
framework="$root/target/client-core/apple/ScreenroomClientCore.xcframework/Info.plist"
binding="$root/target/client-core/bindings/ScreenroomClientCore.swift"
stamp="$root/apps/apple/build/client-core-inputs.sha"
compiler=${RUSTC:-${CARGO_BUILD_RUSTC:-rustc}}
export SCREENROOM_APPLE_BUILD_STD_TARGETS=${SCREENROOM_APPLE_BUILD_STD_TARGETS:-aarch64-apple-ios-sim x86_64-apple-ios x86_64-apple-tvos}
inputs=$({
    find crates/screenroom-client-core crates/screenroom-proto crates/screenroom-observability proto -type f | LC_ALL=C sort | while IFS= read -r file; do shasum -a 256 "$file"; done
    for file in Cargo.toml Cargo.lock rust-toolchain.toml rust-toolchain .cargo/config .cargo/config.toml scripts/build-client-core.sh scripts/patch-swift-cancellation.py apps/apple/scripts/prepare-core.sh; do
        if [ -f "$file" ]; then shasum -a 256 "$file"; fi
    done
    rustup show active-toolchain
    rustc -vV
    rustc +nightly -vV
    printf 'selected-rustc=%s\n' "$(command -v "$compiler")"
    shasum -a 256 "$(command -v "$compiler")"
    "$compiler" -vV
    sysroot=$("$compiler" --print sysroot)
    printf 'sysroot=%s\n' "$sysroot"
    if [ -f "$sysroot/bin/rustc" ]; then shasum -a 256 "$sysroot/bin/rustc"; fi
    nightly_sysroot=$(rustc +nightly --print sysroot)
    printf 'nightly-sysroot=%s\n' "$nightly_sysroot"
    if [ -f "$nightly_sysroot/bin/rustc" ]; then shasum -a 256 "$nightly_sysroot/bin/rustc"; fi
    for wrapper in "${RUSTC_WRAPPER:-}" "${RUSTC_WORKSPACE_WRAPPER:-}" "${CARGO_BUILD_RUSTC_WRAPPER:-}" "${CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER:-}"; do
        if [ -n "$wrapper" ]; then shasum -a 256 "$(command -v "$wrapper")"; fi
    done
    cargo --version
    xcode-select -p
    xcodebuild -version
    xcrun clang --version
    for sdk in macosx iphoneos iphonesimulator appletvos appletvsimulator; do
        xcrun --sdk "$sdk" --show-sdk-path
        xcrun --sdk "$sdk" --show-sdk-version
    done
    env | LC_ALL=C sort | grep -E '^(SCREENROOM_CORE_PROFILE|SCREENROOM_APPLE_TARGETS|SCREENROOM_APPLE_BUILD_STD_TARGETS|RUSTUP_TOOLCHAIN|RUSTC|RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_RUSTFLAGS|CARGO_TARGET_[A-Z0-9_]+_RUSTFLAGS|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_BUILD_RUSTC|CARGO_BUILD_RUSTC_WRAPPER|CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER|DEVELOPER_DIR|SDKROOT|MACOSX_DEPLOYMENT_TARGET|IPHONEOS_DEPLOYMENT_TARGET|TVOS_DEPLOYMENT_TARGET)=' || true
} | shasum -a 256)
artifacts() {
    if [ -f "$framework" ] && [ -f "$binding" ]; then
        find "$(dirname "$framework")" -type f | LC_ALL=C sort | while IFS= read -r file; do shasum -a 256 "$file"; done
        shasum -a 256 "$binding" "${binding%.swift}FFI.h" "${binding%.swift}FFI.modulemap"
    fi
}
current=$(printf '%s\n%s\n' "$inputs" "$(artifacts)" | shasum -a 256)
if [ ! -f "$stamp" ] || [ "$(cat "$stamp")" != "$current" ]; then
    if [ "${SCREENROOM_CORE_PREPARED:-0}" = 1 ]; then echo 'Prepared Apple core changed; rerun preparation before E2E.' >&2; exit 1; fi
    bash "$root/scripts/build-client-core.sh" apple
    mkdir -p "$(dirname "$stamp")"
    printf '%s\n%s\n' "$inputs" "$(artifacts)" | shasum -a 256 > "$stamp"
fi
