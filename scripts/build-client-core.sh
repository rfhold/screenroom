#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
HOST_OS=$(uname -s)
case "$HOST_OS" in
  Darwin) LIB_SUFFIX=dylib; NDK_HOST=darwin-x86_64; DEFAULT_ANDROID_SDK="$HOME/Library/Android/sdk";;
  Linux) LIB_SUFFIX=so; NDK_HOST=linux-x86_64; DEFAULT_ANDROID_SDK="$HOME/Android/Sdk";;
  *) echo "unsupported build host: $HOST_OS (supported: Darwin, Linux)" >&2; exit 2;;
esac
PROFILE=${SCREENROOM_CORE_PROFILE:-debug}
ARGS=()
if [ "$PROFILE" = release ]; then ARGS+=(--release); elif [ "$PROFILE" != debug ]; then echo "profile must be debug or release" >&2; exit 2; fi
OUT="$ROOT/target/client-core"
mkdir -p "$OUT"
native() { cargo build --locked -p screenroom-client-core --lib ${ARGS[@]+"${ARGS[@]}"}; }
bindings() {
  native
  cargo run --locked -p screenroom-client-core --features bindgen --bin screenroom-uniffi-bindgen -- generate \
    --library "target/$PROFILE/libscreenroom_client_core.$LIB_SUFFIX" --language swift --language kotlin \
    --out-dir "$OUT/bindings" --config crates/screenroom-client-core/uniffi.toml --no-format
  python3 scripts/patch-swift-cancellation.py "$OUT/bindings"
}
wasm() {
  local expected actual bindgen
  expected=$(awk '
    /^\[\[package\]\]$/ { selected = 0 }
    /^name = "wasm-bindgen"$/ { selected = 1 }
    selected && /^version = / { gsub(/"/, "", $3); print $3 }
  ' Cargo.lock)
  if [[ ! "$expected" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then echo "Cannot determine a unique wasm-bindgen version from Cargo.lock" >&2; return 2; fi
  if [ -n "${WASM_BINDGEN:-}" ]; then bindgen="$WASM_BINDGEN";
  elif [ -x "$OUT/tools/bin/wasm-bindgen" ]; then bindgen="$OUT/tools/bin/wasm-bindgen";
  else bindgen=wasm-bindgen; fi
  actual=$("$bindgen" --version 2>/dev/null) || actual=unavailable
  if [ "$actual" != "wasm-bindgen $expected" ]; then
    echo "wasm-bindgen CLI mismatch: '$bindgen' reports '$actual'; Cargo.lock requires $expected." >&2
    echo "Install locally: cargo install --locked --version $expected wasm-bindgen-cli --root \"$OUT/tools\"" >&2
    if [ -n "${WASM_BINDGEN:-}" ]; then echo "Unset WASM_BINDGEN or point it to the matching executable." >&2; fi
    return 2
  fi
  cargo build --locked -p screenroom-client-core --lib --target wasm32-unknown-unknown ${ARGS[@]+"${ARGS[@]}"}
  "$bindgen" "target/wasm32-unknown-unknown/$PROFILE/screenroom_client_core.wasm" --target web --out-dir "$OUT/wasm"
}
apple() {
  [ "$HOST_OS" = Darwin ] || { echo "Apple builds require a macOS host" >&2; exit 2; }
  bindings
  local targets=${SCREENROOM_APPLE_TARGETS:-"aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-tvos aarch64-apple-tvos-sim x86_64-apple-tvos"}
  for target in $targets; do
    case " ${SCREENROOM_APPLE_BUILD_STD_TARGETS:-x86_64-apple-tvos} " in
      *" $target "*) cargo +nightly build --locked -Zbuild-std -p screenroom-client-core --lib --target "$target" ${ARGS[@]+"${ARGS[@]}"};;
      *) cargo build --locked -p screenroom-client-core --lib --target "$target" ${ARGS[@]+"${ARGS[@]}"};;
    esac
  done
  local headers="$OUT/apple/headers"
  mkdir -p "$headers"
  cp "$OUT/bindings/ScreenroomClientCoreFFI.h" "$headers/"
  cp "$OUT/bindings/ScreenroomClientCoreFFI.modulemap" "$headers/module.modulemap"
  local xcargs=()
  for platform in macos ios-device ios-simulator tvos-device tvos-simulator; do
    local inputs=()
    case "$platform" in
      macos) group="aarch64-apple-darwin x86_64-apple-darwin";;
      ios-device) group="aarch64-apple-ios";;
      ios-simulator) group="aarch64-apple-ios-sim x86_64-apple-ios";;
      tvos-device) group="aarch64-apple-tvos";;
      tvos-simulator) group="aarch64-apple-tvos-sim x86_64-apple-tvos";;
    esac
    for target in $group; do
      case " $targets " in *" $target "*) inputs+=("$ROOT/target/$target/$PROFILE/libscreenroom_client_core.a");; esac
    done
    if [ "${#inputs[@]}" -gt 0 ]; then
      mkdir -p "$OUT/apple/$platform"
      lipo -create "${inputs[@]}" -output "$OUT/apple/$platform/libscreenroom_client_core.a"
      xcargs+=(-library "$OUT/apple/$platform/libscreenroom_client_core.a" -headers "$headers")
    fi
  done
  # xcodebuild cannot overwrite an existing XCFramework; restrict removal to our generated artifact.
  if [ -d "$OUT/apple/ScreenroomClientCore.xcframework" ]; then rm -r "$OUT/apple/ScreenroomClientCore.xcframework"; fi
  xcodebuild -create-xcframework ${xcargs[@]+"${xcargs[@]}"} -output "$OUT/apple/ScreenroomClientCore.xcframework"
}
android() {
  if [ "$HOST_OS" = Linux ] && [ "$(uname -m)" != x86_64 ]; then echo "unsupported Android NDK host: Linux $(uname -m) (requires x86_64)" >&2; exit 2; fi
  bindings
  local sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$DEFAULT_ANDROID_SDK}}
  local ndk=${ANDROID_NDK_HOME:-$sdk/ndk/29.0.14206865}
  local bin="$ndk/toolchains/llvm/prebuilt/$NDK_HOST/bin"
  [ -x "$bin/aarch64-linux-android26-clang" ] || { echo "Set ANDROID_NDK_HOME to NDK r29 (or compatible NDK)" >&2; exit 1; }
  for target in aarch64-linux-android x86_64-linux-android; do
    if [ "$target" = aarch64-linux-android ]; then abi=arm64-v8a; compiler=aarch64-linux-android26-clang; key=AARCH64_LINUX_ANDROID; else abi=x86_64; compiler=x86_64-linux-android26-clang; key=X86_64_LINUX_ANDROID; fi
    env "CARGO_TARGET_${key}_LINKER=$bin/$compiler" "CC_${target//-/_}=$bin/$compiler" "AR_${target//-/_}=$bin/llvm-ar" \
      cargo build --locked -p screenroom-client-core --lib --target "$target" ${ARGS[@]+"${ARGS[@]}"}
    mkdir -p "$OUT/android/jniLibs/$abi"
    cp "target/$target/$PROFILE/libscreenroom_client_core.so" "$OUT/android/jniLibs/$abi/"
  done
}
case "${1:-native}" in
  native) native;; bindings) bindings;; wasm) wasm;; apple) apple;; android) android;;
  *) echo "usage: $0 native|bindings|wasm|apple|android" >&2; exit 2;;
esac
