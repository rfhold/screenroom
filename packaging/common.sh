#!/usr/bin/env bash

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
targets=(linux-deb-amd64 linux-deb-arm64 linux-arch-amd64 macos ios tvos android-tv android)

die() { printf '%s: %s\n' "${0##*/}" "$*" >&2; exit 1; }

need() {
    local tool
    for tool; do command -v "$tool" >/dev/null || die "required tool not found: $tool"; done
}

release_inputs() {
    [[ $1 =~ ^(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]?)\.(0|[1-9][0-9]?)$ ]] ||
        die 'use a numeric MAJOR.MINOR.PATCH version (minor and patch at most 99)'
    [[ $2 =~ ^[1-9][0-9]{0,3}$ ]] || die 'use a build number from 1 through 9999'
}

artifact_metadata() {
    local version=$2 build=$3
    signature=unsigned
    case "$1" in
        linux-deb-amd64) architecture=amd64; artifact="screenroom_${version}-${build}_amd64.deb" ;;
        linux-deb-arm64) architecture=arm64; artifact="screenroom_${version}-${build}_arm64.deb" ;;
        linux-arch-amd64) architecture=x86_64; artifact="screenroom-${version}-${build}-x86_64.pkg.tar.zst" ;;
        macos) architecture=arm64,x86_64; artifact="screenroom-macos-${version}-${build}.zip"; signature=ad-hoc ;;
        ios|tvos) architecture=arm64; artifact="screenroom-$1-${version}-${build}.xcarchive.zip" ;;
        android|android-tv) architecture=arm64-v8a,x86_64; artifact="screenroom-$1-${version}-${build}.apk" ;;
        *) die "unknown package target: $1 (choose ${targets[*]})" ;;
    esac
}

sha256() { shasum -a 256 <"$1" | awk '{print $1}'; }
size() { wc -c <"$1" | tr -d '[:space:]'; }

android_module() {
    case "$1" in
        android) printf '%s\n' mobile ;;
        android-tv) printf '%s\n' tv ;;
        *) die "unknown Android target: $1" ;;
    esac
}
