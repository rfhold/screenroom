#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
target=$1 version=$2 build=$3 output=$4
module=$(android_module "$target")
app_id=tv.screenroom
if [[ $module == tv ]]; then app_id=tv.screenroom.tv; fi
case "$(uname -s)" in
    Darwin) default_sdk="$HOME/Library/Android/sdk" ;;
    Linux) default_sdk="$HOME/Android/Sdk" ;;
    *) die 'use macOS or Linux with the Android SDK' ;;
esac
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$default_sdk}}
tools="$sdk/build-tools/${ANDROID_BUILD_TOOLS_VERSION:-36.0.0}"
[[ -x $tools/aapt && -x $tools/apksigner ]] || die 'install Android build-tools 36.0.0 or set ANDROID_BUILD_TOOLS_VERSION'
cd "$root/apps/android"
SCREENROOM_CORE_PROFILE=release ./gradlew --no-daemon ":$module:assembleRelease" \
    "-PscreenroomVersion=$version" "-PscreenroomBuild=$build"
apk="$module/build/outputs/apk/release/$module-release-unsigned.apk"
unzip -tq "$apk"
info=$("$tools/aapt" dump badging "$apk")
grep -Fq "package: name='$app_id' versionCode='$build' versionName='$version'" <<<"$info" ||
    die 'unexpected Android application ID or release version'
grep -Fxq "native-code: 'arm64-v8a' 'x86_64'" <<<"$info" || die 'unexpected Android ABIs'
for abi in arm64-v8a x86_64; do
    unzip -p "$apk" "lib/$abi/libscreenroom_client_core.so" >/dev/null
done
if "$tools/apksigner" verify "$apk" >/dev/null 2>&1; then
    die 'expected an unsigned Android APK'
fi
cp "$apk" "$output"
