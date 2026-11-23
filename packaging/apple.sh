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
signer=-
if [[ $target == macos ]]; then
    archs='arm64 x86_64'
    if [[ ${SCREENROOM_PACKAGE_LOCAL:-0} != 1 ]]; then
        identity=${SCREENROOM_MACOS_SIGN_IDENTITY:-}
        profile=${SCREENROOM_MACOS_NOTARY_PROFILE:-}
        [[ -n $identity && -n $profile ]] ||
            die 'set SCREENROOM_MACOS_SIGN_IDENTITY and SCREENROOM_MACOS_NOTARY_PROFILE'
        signer=$(/usr/bin/security find-identity -v -p codesigning | awk -F '"' -v identity="$identity" '
            $2 ~ /^Developer ID Application: / {
                split($1, fields, " ")
                hash = fields[2]
                if ($2 == identity || tolower(hash) == tolower(identity)) selected[hash] = 1
            }
            END {
                for (hash in selected) { result = hash; count++ }
                if (count == 1) print result; else exit 1
            }
        ') || die 'select exactly one valid Developer ID Application identity'
    fi
fi
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
for field in "CFBundleIdentifier:tv.screenroom" "CFBundleShortVersionString:$version" \
    "CFBundleVersion:$build" "DTPlatformName:$sdk"; do
    [[ $(/usr/libexec/PlistBuddy -c "Print :${field%%:*}" "$plist") == "${field#*:}" ]] ||
        die "unexpected Apple metadata: ${field%%:*}"
done
actual=$(lipo -archs "$bin" | tr ' ' '\n' | LC_ALL=C sort)
expected=$(printf '%s\n' $archs | LC_ALL=C sort)
[[ $actual == "$expected" ]] || die "unexpected Apple architectures: $actual"

if [[ $target == macos ]]; then
    sign=(/usr/bin/codesign --force --sign "$signer" --entitlements Resources/macOS/Screenroom.entitlements)
    if [[ $signer != - ]]; then sign+=(--options runtime --timestamp); fi
    "${sign[@]}" "$app"
    /usr/bin/codesign --verify --deep --strict "$app"
    if [[ $signer != - ]]; then
        ditto -c -k --keepParent "$app" "$work/submission.zip"
        reply=$(xcrun notarytool submit "$work/submission.zip" --keychain-profile "$profile" \
            --wait --timeout 12m --output-format json) ||
            die 'Apple notarization did not complete'
        jq -e '.status == "Accepted"' <<<"$reply" >/dev/null ||
            die 'Apple did not accept the notarization submission'
        xcrun stapler staple "$app"
        xcrun stapler validate "$app"
        /usr/sbin/spctl --assess --type execute -v "$app"
    fi
    ditto -c -k --sequesterRsrc --keepParent "$app" "$output"
else
    [[ $(codesign -dv "$app" 2>&1 || true) == *'code object is not signed at all'* ]] ||
        die 'expected an unsigned device archive'
    ditto -c -k --sequesterRsrc --keepParent "$archive" "$output"
fi
unzip -tq "$output"
