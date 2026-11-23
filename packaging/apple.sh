#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
target=$1 action=$2
case "$target" in
    macos) scheme=Screenroom-macOS; platform=macOS; sdk=macosx ;;
    ios) scheme=Screenroom-iOS; platform=iOS; sdk=iphoneos ;;
    tvos) scheme=Screenroom-tvOS; platform=tvOS; sdk=appletvos ;;
    *) die "unknown Apple target: $target" ;;
esac
[[ $(uname -s) == Darwin ]] || die 'use macOS with Xcode and XcodeGen'
need xcodegen xcodebuild
cd "$root/apps/apple"

if [[ $action == package ]]; then
    export SCREENROOM_CORE_PROFILE=release
    export SCREENROOM_APPLE_TARGETS='aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-tvos'
fi
xcodegen generate
args=(-project Screenroom.xcodeproj -scheme "$scheme" CODE_SIGNING_ALLOWED=NO)
case "$action" in
    build|check)
        if [[ $target != macos ]]; then platform="$platform Simulator"; fi
        if [[ $action == check ]]; then action=analyze; fi
        exec xcodebuild "${args[@]}" -destination "generic/platform=$platform" \
            -derivedDataPath "build/cli-$target" "$action" ;;
    package) ;;
    *) die "unknown Apple action: $action" ;;
esac

version=$3 build=$4 work=$5 output="$5/$6"
archive="$work/Screenroom.xcarchive"
archs=arm64
if [[ $target == macos ]]; then archs='arm64 x86_64'; fi
xcodebuild "${args[@]}" -configuration Release -destination "generic/platform=$platform" \
    -derivedDataPath "build/release-$target" -archivePath "$archive" \
    "ARCHS=$archs" ONLY_ACTIVE_ARCH=NO "MARKETING_VERSION=$version" "CURRENT_PROJECT_VERSION=$build" archive
app="$archive/Products/Applications/Screenroom.app"
if [[ $target == macos ]]; then
    plist="$app/Contents/Info.plist"
    bin="$app/Contents/MacOS/Screenroom"
else
    plist="$app/Info.plist"
    bin="$app/Screenroom"
fi
for field in "CFBundleIdentifier:org.screenroom.$target" "CFBundleShortVersionString:$version" \
    "CFBundleVersion:$build" "DTPlatformName:$sdk"; do
    [[ $(/usr/libexec/PlistBuddy -c "Print :${field%%:*}" "$plist") == "${field#*:}" ]] ||
        die "unexpected Apple metadata: ${field%%:*}"
done
actual=$(lipo -archs "$bin" | tr ' ' '\n' | LC_ALL=C sort)
expected=$(printf '%s\n' $archs | LC_ALL=C sort)
[[ $actual == "$expected" ]] || die "unexpected Apple architectures: $actual"

if [[ $target == macos ]]; then
    codesign --force --sign - --entitlements Resources/macOS/Screenroom.entitlements "$app"
    codesign --verify --deep --strict "$app"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$output"
else
    [[ $(codesign -dv "$app" 2>&1 || true) == *'code object is not signed at all'* ]] ||
        die 'expected an unsigned device archive'
    ditto -c -k --sequesterRsrc --keepParent "$archive" "$output"
fi
unzip -tq "$output"
