#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
source scripts/test-category.sh
configure_apple_test_category
if [ "$apple_test_journey" = playback ]; then
    for name in SCREENROOM_API_ENDPOINT SCREENROOM_BACKEND_DESCRIPTOR SCREENROOM_BACKEND_INSTANCE_ID SCREENROOM_SHOWCASE_ITEM_ID SCREENROOM_SHOWCASE_TITLE SCREENROOM_SHOWCASE_YEAR; do
        if [ -z "${!name:-}" ]; then echo "Playback requires explicit $name" >&2; exit 2; fi
    done
    if [ "${SCREENROOM_NATIVE_PLAYBACK_RELEASED:-0}" != 1 ]; then
        echo 'Playback requires SCREENROOM_NATIVE_PLAYBACK_RELEASED=1 from the coordinator' >&2; exit 2
    fi
    if [ "${SCREENROOM_BACKEND_ATTACH:-}" != 1 ] ||
        [ "${SCREENROOM_BACKEND_PROJECT:-}" != screenroom-jellyfin-local ] ||
        [ "$SCREENROOM_API_ENDPOINT" != http://127.0.0.1:50052 ]; then
        echo 'Playback requires explicit attachment to screenroom-jellyfin-local at http://127.0.0.1:50052' >&2; exit 2
    fi
    if ! node --input-type=module -e '
        import { readFileSync } from "node:fs";
        const env = process.env;
        const descriptor = JSON.parse(readFileSync(env.SCREENROOM_BACKEND_DESCRIPTOR, "utf8"));
        if (descriptor.instanceID !== env.SCREENROOM_BACKEND_INSTANCE_ID ||
            descriptor.publicOrigin !== env.SCREENROOM_API_ENDPOINT) process.exit(2);
    '; then
        echo 'Playback requires the explicit descriptor-backed backend instance' >&2; exit 2
    fi
fi
case "${E2E_MACOS-0}" in
    0|1) ;;
    *) echo 'E2E_MACOS must be 0 or 1 (or unset)' >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then
    echo "Usage: bash apps/apple/scripts/e2e.sh [ios|ipad|tvos|macos]" >&2
    exit 2
fi
case "${1:-all}" in
    all)
        profiles=(ios ipad tvos)
        if [ "${E2E_MACOS-0}" = 1 ]; then
            if [ "$apple_test_journey" = playback ]; then profiles=(macos "${profiles[@]}"); else profiles+=(macos); fi
        fi
        ;;
    ios|ipad|tvos|macos) profiles=("$1") ;;
    *) echo "Unknown Apple E2E profile: $1" >&2; exit 2 ;;
esac
for profile in "${profiles[@]}"; do
    if [ "$profile" = macos ] && [ "${SCREENROOM_MACOS_GUI_READY:-0}" != 1 ]; then
        echo 'macOS GUI testing requires fresh user permission/readiness; set SCREENROOM_MACOS_GUI_READY=1 only after that approval.' >&2
        exit 2
    fi
done

timeout=$(command -v timeout || command -v gtimeout || true)
if [ -z "$timeout" ] || ! command -v jq >/dev/null; then
    echo "Install GNU coreutils and jq: brew install coreutils jq" >&2
    exit 1
fi
mkdir -p build/e2e/cache build/e2e/runs
if [ -n "${E2E_APPLE_RUN_DIR:-}" ]; then
    case "$E2E_APPLE_RUN_DIR" in
        /*) run=$E2E_APPLE_RUN_DIR ;;
        *) echo 'E2E_APPLE_RUN_DIR must be an absolute, new directory' >&2; exit 2 ;;
    esac
    mkdir "$run"
else
    run=$(mktemp -d "$PWD/build/e2e/runs/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")
fi
echo "Apple E2E artifacts: $run"
trap 'echo "Apple E2E failed; see $run" >&2' ERR
trap 'exit 130' INT
trap 'exit 143' TERM HUP
root=$(cd ../.. && pwd)
export SCREENROOM_BACKEND_DESCRIPTOR=${SCREENROOM_BACKEND_DESCRIPTOR:-"$root/build/backend-runtime/screenroom-e2e/descriptor.json"}
if [ "$apple_test_journey" = browse ]; then
    export SCREENROOM_BACKEND_ATTACH=1
    export SCREENROOM_API_ENDPOINT=${SCREENROOM_API_ENDPOINT:-http://127.0.0.1:50051}
fi
if [ -n "${SCREENROOM_E2E_API_ENDPOINT:-}" ] && [ "$SCREENROOM_E2E_API_ENDPOINT" != "$SCREENROOM_API_ENDPOINT" ]; then
    echo 'SCREENROOM_E2E_API_ENDPOINT must match SCREENROOM_API_ENDPOINT' >&2; exit 2
fi
bash "$root/scripts/backend.sh" wait >"$run/backend-wait.log" 2>&1
curl -fsS --max-time 10 "$SCREENROOM_API_ENDPOINT/__screenroom_instance" >"$run/backend-instance.json"
instance=$(jq -er '.instanceID' "$run/backend-instance.json")
descriptor_instance=$(jq -er '.instanceID' "$SCREENROOM_BACKEND_DESCRIPTOR")
descriptor_origin=$(jq -er '.publicOrigin' "$SCREENROOM_BACKEND_DESCRIPTOR")
if [ "$instance" != "$descriptor_instance" ] || [ "$SCREENROOM_API_ENDPOINT" != "$descriptor_origin" ] ||
    { [ -n "${SCREENROOM_BACKEND_INSTANCE_ID:-}" ] && [ "$instance" != "$SCREENROOM_BACKEND_INSTANCE_ID" ]; }; then
    echo 'Apple runner requires the existing descriptor-backed shared backend instance' >&2; exit 2
fi
export SCREENROOM_BACKEND_INSTANCE_ID="$instance"
export SCREENROOM_USER_ID=${SCREENROOM_USER_ID:-$(jq -er '.userID' "$SCREENROOM_BACKEND_DESCRIPTOR")}
echo "Attached shared backend $instance at $SCREENROOM_API_ENDPOINT" | tee "$run/backend-lifecycle.log"
export TEST_RUNNER_SCREENROOM_API_ENDPOINT="$SCREENROOM_API_ENDPOINT"
export TEST_RUNNER_SCREENROOM_USER_ID="$SCREENROOM_USER_ID"
export TEST_RUNNER_SCREENROOM_BACKEND_INSTANCE_ID="$instance"
if [ "$apple_test_journey" = playback ]; then
    export TEST_RUNNER_SCREENROOM_E2E_JOURNEY=playback
    export TEST_RUNNER_SCREENROOM_NATIVE_PLAYBACK_RELEASED=1
    export TEST_RUNNER_SCREENROOM_SHOWCASE_ITEM_ID="$SCREENROOM_SHOWCASE_ITEM_ID"
    export TEST_RUNNER_SCREENROOM_SHOWCASE_TITLE="$SCREENROOM_SHOWCASE_TITLE"
    export TEST_RUNNER_SCREENROOM_SHOWCASE_YEAR="$SCREENROOM_SHOWCASE_YEAR"
fi
"$timeout" --foreground -k 10s 1200s bash scripts/prepare-core.sh >"$run/core-build.log" 2>&1
if [ "$apple_test_category" = regression ] || [ "$apple_test_category" = all ]; then
    "$timeout" --foreground -k 10s 120s bash scripts/test-catalog-state.sh >"$run/catalog-state.log" 2>&1
    "$timeout" --foreground -k 10s 120s bash scripts/test-endpoint-policy.sh >"$run/endpoint-policy.log" 2>&1
fi
"$timeout" --foreground -k 10s 60s xcodegen generate --use-cache \
    --cache-path build/e2e/cache/xcodegen >"$run/generate.log" 2>&1

for profile in "${profiles[@]}"; do
    signing=(CODE_SIGNING_ALLOWED=NO)
    case "$profile" in
        ios) scheme=iOS; pattern='^iPhone'; platform='iOS Simulator'; destination=${E2E_IOS_DESTINATION:-} ;;
        ipad) scheme=iOS; pattern='^iPad'; platform='iOS Simulator'; destination=${E2E_IPAD_DESTINATION:-} ;;
        tvos) scheme=tvOS; pattern='^Apple TV'; platform='tvOS Simulator'; destination=${E2E_TVOS_DESTINATION:-} ;;
        macos)
            scheme=macOS; destination='platform=macOS'
            signing=(CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=)
            ;;
    esac
    if [ "$profile" != macos ]; then
            devices=$("$timeout" --foreground -k 10s 30s xcrun simctl list devices available --json)
        candidates=$(jq -r --arg pattern "$pattern" --arg runtime "com.apple.CoreSimulator.SimRuntime.$scheme-" '
            [.devices | to_entries[] | select(.key | startswith($runtime)) | .value[]
             | select(.isAvailable and (.name | test($pattern)))]
            | sort_by(.state != "Booted") | .[].udid' <<<"$devices")
        if [ -z "$destination" ]; then
            id=${candidates%%$'\n'*}
            destination="platform=$platform,id=$id"
        else
            id=${destination#"platform=$platform,id="}
        fi
        if [ -z "$id" ] || [ "$destination" != "platform=$platform,id=$id" ] ||
            ! grep -Fxq "$id" <<<"$candidates"; then
            echo "No matching available simulator for $profile: $destination" >&2
            exit 2
        fi
    fi

    output="$run/$profile"
    mkdir -p "$output"
    if [ "$apple_test_journey" = playback ]; then
        export TEST_RUNNER_SCREENROOM_SHOWCASE_PLATFORM="$profile"
        export TEST_RUNNER_SCREENROOM_SHOWCASE_RUN_ID="$(uuidgen)"
        capture_started=$(date +%s)
    fi
    test_allowance=120
    if [ "$apple_test_journey" = playback ]; then test_allowance=360; fi
    echo "E2E $profile: $destination"
    selectors=()
    for method in "${apple_test_methods[@]}"; do
        selectors+=("-only-testing:Screenroom-$scheme-E2ETests/$apple_test_class/$method")
    done
    if [ "$profile" = tvos ] && { [ "$apple_test_category" = regression ] || [ "$apple_test_category" = all ]; }; then
        selectors+=("-only-testing:Screenroom-$scheme-E2ETests/HomeE2ETests/testSidebarRemoteControls")
    fi
    echo "Category $apple_test_category: ${selectors[*]}" >>"$output/test-selection.log"
    "$timeout" --foreground -k 10s 480s xcodebuild test -project Screenroom.xcodeproj \
        -scheme "Screenroom-$scheme" -destination "$destination" -destination-timeout 60 \
        -derivedDataPath "$PWD/build/e2e/cache/$scheme" "${signing[@]}" "${selectors[@]}" \
        -resultBundlePath "$output/Home.xcresult" -parallel-testing-enabled NO \
        -test-timeouts-enabled YES -maximum-test-execution-time-allowance "$test_allowance" \
        >"$output/test.log" 2>&1
    for report in tests summary; do
        "$timeout" --foreground -k 10s 60s xcrun xcresulttool get test-results "$report" \
            --path "$output/Home.xcresult" >"$output/test-results-$report.json" 2>"$output/test-results-$report.log"
    done
    node scripts/verify-test-results.mjs "$output/test-results-tests.json" "$output/test-results-summary.json" \
        "${selectors[@]}" >"$output/test-verification.json" 2>"$output/test-verification.log"
    "$timeout" --foreground -k 10s 60s xcrun xcresulttool export attachments \
        --path "$output/Home.xcresult" --output-path "$output/screenshots" \
        >"$output/export.log" 2>&1
    if [ "$apple_test_journey" = playback ]; then
        node scripts/playback-showcase-extract.mjs "$output/screenshots" "$output/playback" \
            "$profile" "$scheme" "$capture_started" "$TEST_RUNNER_SCREENROOM_SHOWCASE_RUN_ID"
    fi
    echo "PASS $profile: $output"
done
