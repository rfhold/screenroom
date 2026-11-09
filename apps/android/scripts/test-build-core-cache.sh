#!/usr/bin/env bash
set -euo pipefail
ANDROID_ROOT=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$ANDROID_ROOT/build"
WORK=$(mktemp -d "$ANDROID_ROOT/build/cache-key-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/apps/android/scripts" "$WORK/scripts" "$WORK/crates/screenroom-client-core" "$WORK/crates/screenroom-proto" "$WORK/proto" "$WORK/bin" "$WORK/sdk" "$WORK/sdk-other"
cp "$ANDROID_ROOT/scripts/build-core.sh" "$WORK/apps/android/scripts/"
touch "$WORK/Cargo.toml" "$WORK/Cargo.lock"
cat > "$WORK/bin/rustc" <<'EOF'
#!/usr/bin/env bash
echo "rustc ${CACHE_TEST_RUST_ID:-one} ${RUSTUP_TOOLCHAIN:-default} $*"
EOF
cat > "$WORK/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo 'cargo test identity'
EOF
chmod +x "$WORK/bin/rustc" "$WORK/bin/cargo"
for ndk in ndk ndk-other; do
  for host in darwin-x86_64 linux-x86_64; do
    mkdir -p "$WORK/$ndk/toolchains/llvm/prebuilt/$host/bin"
    for compiler in aarch64-linux-android26-clang x86_64-linux-android26-clang llvm-ar; do
      cp "$WORK/bin/cargo" "$WORK/$ndk/toolchains/llvm/prebuilt/$host/bin/$compiler"
    done
  done
  echo 'Pkg.Revision = 29.0.14206865' > "$WORK/$ndk/source.properties"
done
cat > "$WORK/scripts/build-client-core.sh" <<'EOF'
#!/usr/bin/env bash
set -eu
echo build >> builds
mkdir -p target/client-core/bindings/org/screenroom/core target/client-core/android/jniLibs/{arm64-v8a,x86_64}
for file in target/client-core/bindings/org/screenroom/core/screenroom_client_core.kt target/client-core/android/jniLibs/{arm64-v8a,x86_64}/libscreenroom_client_core.so; do echo product > "$file"; done
EOF
export PATH="$WORK/bin:$PATH" ANDROID_HOME="$WORK/sdk" ANDROID_NDK_HOME="$WORK/ndk" SCREENROOM_CORE_PROFILE=debug
unset RUSTC RUSTUP_TOOLCHAIN RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTC CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS SCREENROOM_CORE_PREPARED
check_builds() {
  bash "$WORK/apps/android/scripts/build-core.sh"
  actual=$(wc -l < "$WORK/builds" | tr -d ' ')
  [ "$actual" = "$1" ] || { echo "Expected $1 builds, got $actual" >&2; exit 1; }
}
check_builds 1
check_builds 1
export SCREENROOM_CORE_PROFILE=release
check_builds 2
export RUSTUP_TOOLCHAIN=other-toolchain
check_builds 3
export CACHE_TEST_RUST_ID=two
check_builds 4
echo 'Pkg.Revision = changed' > "$WORK/ndk/source.properties"
check_builds 5
export ANDROID_NDK_HOME="$WORK/ndk-other"
check_builds 6
export ANDROID_HOME="$WORK/sdk-other"
check_builds 7
cp "$WORK/bin/rustc" "$WORK/bin/other-rustc"
export RUSTC="$WORK/bin/other-rustc"
check_builds 8
check_builds 8
builds=8
reject_prepared() {
  if SCREENROOM_CORE_PREPARED=1 bash "$WORK/apps/android/scripts/build-core.sh" >"$WORK/rejected.log" 2>&1; then echo 'Expected stale prepared cache rejection' >&2; exit 1; fi
  [ "$(wc -l < "$WORK/builds" | tr -d ' ')" = "$builds" ] || { echo 'Prepared mode rebuilt products' >&2; exit 1; }
}
rebuild_changed() {
  reject_prepared
  builds=$((builds + 1))
  check_builds "$builds"
  SCREENROOM_CORE_PREPARED=1 check_builds "$builds"
}
for file in "$WORK/target/client-core/bindings/org/screenroom/core/screenroom_client_core.kt" "$WORK/target/client-core/android/jniLibs/arm64-v8a/libscreenroom_client_core.so" "$WORK/target/client-core/android/jniLibs/x86_64/libscreenroom_client_core.so"; do
  echo corrupt > "$file"
  rebuild_changed
  rm "$file"
  rebuild_changed
done
cp "$WORK/bin/rustc" "$WORK/bin/selected-rustc"
export RUSTC="$WORK/bin/selected-rustc"
rebuild_changed
echo '# compiler changed' >> "$RUSTC"
rebuild_changed
unset RUSTC
export CARGO_BUILD_RUSTC="$WORK/bin/selected-rustc"
rebuild_changed
for variable in RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER; do
  cp "$WORK/bin/cargo" "$WORK/bin/$variable"
  export "$variable=$WORK/bin/$variable"
  rebuild_changed
  echo '# wrapper changed' >> "$WORK/bin/$variable"
  rebuild_changed
done
for variable in RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS; do
  export "$variable=-Copt-level=1"
  rebuild_changed
done
echo 'Android core cache: initial build, warm reuse, profile, toolchain, compiler identity/path, NDK version/path, SDK path passed'
echo 'Android prepared cache: Kotlin/JNI tamper and removal, compiler/wrapper selection and content, and all Cargo Rust flags passed'
