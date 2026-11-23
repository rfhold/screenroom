#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
cd "$root"
[[ $# == 2 ]] || die 'usage: build.sh build|check TARGET'
case "$1:$2" in
    build:core) exec bash scripts/build-client-core.sh native ;;
    build:core-bindings|build:core-wasm|build:core-apple|build:core-android)
        exec bash scripts/build-client-core.sh "${2#core-}" ;;
    check:core) exec cargo test -p screenroom-client-core --lib --locked ;;
    build:backend) exec cargo build --locked --bins ;;
    check:backend) exec cargo check --locked --bins ;;
    build:linux|check:linux)
        exec cargo "$1" --manifest-path apps/linux/Cargo.toml --locked ;;
    build:web|build:website|check:web|check:website)
        exec pnpm --filter "@screenroom/$2" "$1" ;;
    build:android|build:android-tv|check:android|check:android-tv)
        module=$(android_module "$2")
        cd apps/android
        case "$1" in
            build) exec ./gradlew --no-daemon ":$module:assembleDebug" ;;
            check) exec ./gradlew --no-daemon ":$module:lintDebug" ;;
        esac ;;
    build:macos|build:ios|build:tvos|check:macos|check:ios|check:tvos)
        exec bash packaging/apple.sh "$2" "$1" ;;
    check:release) exec bash packaging/tests/release.sh ;;
    *) die "unknown command or target: $1 $2" ;;
esac
