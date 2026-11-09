#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$ROOT"
STAMP="$ROOT/target/client-core/android/.inputs.sha256"
case "$(uname -s)" in
  Darwin) DEFAULT_SDK="$HOME/Library/Android/sdk"; NDK_HOST=darwin-x86_64;;
  Linux) DEFAULT_SDK="$HOME/Android/Sdk"; NDK_HOST=linux-x86_64;;
  *) echo 'Unsupported Android build host' >&2; exit 2;;
esac
SDK=$(cd "${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$DEFAULT_SDK}}" && pwd -P)
NDK=$(cd "${ANDROID_NDK_HOME:-$SDK/ndk/29.0.14206865}" && pwd -P)
RUST_COMPILER=${RUSTC:-${CARGO_BUILD_RUSTC:-rustc}}
HASH=$(
  set -e
  find "$ROOT/crates/screenroom-client-core" "$ROOT/crates/screenroom-proto" "$ROOT/proto" -type f ! -path '*/target/*' -print | sort | while IFS= read -r file; do shasum -a 256 "$file"; done
  shasum -a 256 "$ROOT/Cargo.lock" "$ROOT/Cargo.toml" "$ROOT/scripts/build-client-core.sh" "$ROOT/apps/android/scripts/build-core.sh"
  for file in rust-toolchain rust-toolchain.toml .cargo/config .cargo/config.toml; do
    if [ -f "$file" ]; then shasum -a 256 "$file"; fi
  done
  printf 'profile=%s\nsdk=%s\nndk=%s\nhost=%s\n' "${SCREENROOM_CORE_PROFILE:-debug}" "$SDK" "$NDK" "$(uname -sm)"
  printf 'rustc=%s\n' "$(command -v "$RUST_COMPILER")"
  shasum -a 256 "$(command -v "$RUST_COMPILER")"
  "$RUST_COMPILER" -Vv
  sysroot=$("$RUST_COMPILER" --print sysroot)
  printf 'sysroot=%s\n' "$sysroot"
  if [ -f "$sysroot/bin/rustc" ]; then shasum -a 256 "$sysroot/bin/rustc"; fi
  for wrapper in "${RUSTC_WRAPPER:-}" "${RUSTC_WORKSPACE_WRAPPER:-}" "${CARGO_BUILD_RUSTC_WRAPPER:-}" "${CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER:-}"; do
    if [ -n "$wrapper" ]; then shasum -a 256 "$(command -v "$wrapper")"; fi
  done
  printf 'cargo=%s\n' "$(command -v cargo)"
  cargo -Vv
  env | LC_ALL=C sort | grep -E '^(RUSTUP_TOOLCHAIN|RUSTC|RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_RUSTFLAGS|CARGO_TARGET_[A-Z0-9_]+_RUSTFLAGS|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_BUILD_RUSTC|CARGO_BUILD_RUSTC_WRAPPER|CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER)=' || true
  shasum -a 256 "$NDK/source.properties"
  for compiler in aarch64-linux-android26-clang x86_64-linux-android26-clang llvm-ar; do
    "$NDK/toolchains/llvm/prebuilt/$NDK_HOST/bin/$compiler" --version
  done
)
artifacts() {
  for file in target/client-core/bindings/org/screenroom/core/screenroom_client_core.kt target/client-core/android/jniLibs/{arm64-v8a,x86_64}/libscreenroom_client_core.so; do
    [ -f "$file" ] || return 1
    shasum -a 256 "$file" || return 1
  done
}
products=$(artifacts) || products=missing
current=$(printf '%s\n%s\n' "$HASH" "$products" | shasum -a 256)
if [ ! -f "$STAMP" ] || [ "$(cat "$STAMP")" != "$current" ]; then
  if [ "${SCREENROOM_CORE_PREPARED:-0}" = 1 ]; then echo 'Prepared Android core changed; rerun preparation before E2E.' >&2; exit 1; fi
  bash "$ROOT/scripts/build-client-core.sh" android
  products=$(artifacts)
  printf '%s\n%s\n' "$HASH" "$products" | shasum -a 256 > "$STAMP"
fi
