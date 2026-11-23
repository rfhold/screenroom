#!/usr/bin/env bash
set -euo pipefail

case "${E2E_CATEGORY-all}" in
  happy-path) categories=(happy-path); methods=(happyPath) ;;
  retry) categories=(retry); methods=(retry) ;;
  regression) categories=(regression); methods=(regression) ;;
  all) categories=(happy-path retry regression); methods=(happyPath retry regression) ;;
  *) echo 'Invalid E2E_CATEGORY; expected happy-path, retry, regression, or all' >&2; exit 2 ;;
esac
journey=${SCREENROOM_E2E_JOURNEY-browse}
case "$journey" in
  browse) ;;
  playback)
    [ "${E2E_CATEGORY-all}" = happy-path ] || { echo 'Playback journey requires E2E_CATEGORY=happy-path' >&2; exit 2; }
    methods=(playbackShowcase)
    for selector in SCREENROOM_SHOWCASE_ITEM_ID SCREENROOM_SHOWCASE_TITLE SCREENROOM_SHOWCASE_YEAR SCREENROOM_BACKEND_INSTANCE_ID; do
      [ -n "${!selector:-}" ] || { echo "Playback journey requires $selector" >&2; exit 2; }
    done
    [[ "$SCREENROOM_SHOWCASE_YEAR" =~ ^[0-9]{4}$ ]] || { echo 'Invalid showcase year' >&2; exit 2; }
    [ "${SCREENROOM_API_ENDPOINT:-}" = http://127.0.0.1:50052 ] || [ "${SCREENROOM_API_ENDPOINT:-}" = http://localhost:50052 ] \
      || { echo 'Playback requires the attached real endpoint on port 50052' >&2; exit 2; }
    [ "${SCREENROOM_BACKEND_ATTACH:-}" = 1 ] && [ "${SCREENROOM_BACKEND_PROJECT:-}" = screenroom-jellyfin-local ] \
      || { echo 'Playback requires the real attached backend project' >&2; exit 2; }
    ;;
  *) echo 'Invalid SCREENROOM_E2E_JOURNEY; expected browse or playback' >&2; exit 2 ;;
esac
capture=false
for category in "${categories[@]}"; do [ "$category" != happy-path ] || capture=true; done

ROOT=$(cd "$(dirname "$0")/.." && pwd)
case "${1:-}" in
  '') profiles=(mobile tv) ;;
  mobile|tv) profiles=("$1") ;;
  *) echo 'Usage: bash apps/android/scripts/e2e.sh [mobile|tv]' >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { echo 'Expected at most one profile' >&2; exit 2; }

OUT="$ROOT/build/e2e"
profile_prefix=${SCREENROOM_E2E_SETTINGS_NAMESPACE:-${SCREENROOM_E2E_PROFILE_PREFIX:-android-$(uuidgen | tr '[:upper:]' '[:lower:]')}}
[[ "$profile_prefix" =~ ^[a-zA-Z0-9_-]{1,90}$ ]] || { echo 'Invalid E2E profile prefix' >&2; exit 2; }
flows=(home sidebar library search detail)
playback_stage=
playback_run=
if [ "$journey" = playback ]; then flows=(); fi
mkdir -p "$OUT"
for profile in "${profiles[@]}"; do
  for flow in "${flows[@]}"; do
    rm -f "$OUT/$profile-$flow.png" "$OUT/$profile-$flow.png.tmp"
  done
done

export JAVA_HOME=${JAVA_HOME:-/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home}
export ANDROID_HOME=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}
if [ -z "${ANDROID_AVD_HOME:-}" ] && [ -d "$ROOT/build/avd" ]; then
  export ANDROID_AVD_HOME="$ROOT/build/avd"
fi
ADB="$ANDROID_HOME/platform-tools/adb"
EMULATOR="$ANDROID_HOME/emulator/emulator"
fail() { echo "Android E2E: $*" >&2; exit 1; }
command -v timeout >/dev/null || fail 'Install coreutils timeout before this suite'
timeout() { command timeout --foreground "$@"; }
command -v file >/dev/null || fail 'file is required to validate screenshots'
[ -x "$JAVA_HOME/bin/java" ] || fail "JDK 17 not found: $JAVA_HOME"
[ -x "$ADB" ] || fail "adb not found: $ADB"
[ -x "$EMULATOR" ] || fail "emulator not found: $EMULATOR"
[ -f "$ANDROID_HOME/platforms/android-37/android.jar" ] \
  || [ -f "$ANDROID_HOME/platforms/android-37.0/android.jar" ] \
  || fail 'Android SDK platform 37 is required'
"$JAVA_HOME/bin/java" -version 2>&1 | grep -q 'version "17\.' || fail 'JAVA_HOME must select JDK 17'

owned_pid=
owned_reverses=()
verified_serials=()
verified_packages=()
source "$ROOT/scripts/instrumentation.sh"
cleanup() {
  status=$?
  stop_owned_instrumentation
  if [ -n "$playback_stage" ]; then rm -rf "$playback_stage"; fi
  if [ -n "$playback_run" ]; then
    for ((index = 0; index < ${#verified_serials[@]}; index++)); do
      timeout 15 "$ADB" -s "${verified_serials[$index]}" shell run-as "${verified_packages[$index]}" \
        rm -rf "files/screenshots/playback/$playback_run" >/dev/null 2>&1 || true
    done
  fi
  for reverse_serial in "${owned_reverses[@]}"; do "$ADB" -s "$reverse_serial" reverse --remove "tcp:$api_port" >/dev/null 2>&1 || true; done
  for profile in "${profiles[@]}"; do
    for flow in "${flows[@]}"; do
      rm -f "$OUT/$profile-$flow.png.tmp"
      if [ "$status" -ne 0 ]; then rm -f "$OUT/$profile-$flow.png"; fi
    done
  done
  if [ "$status" -ne 0 ]; then
    for ((index = 0; index < ${#verified_serials[@]}; index++)); do
      for flow in "${flows[@]}"; do
        timeout 15 "$ADB" -s "${verified_serials[$index]}" shell run-as "${verified_packages[$index]}" \
          rm -f "files/screenshots/$flow.png" >/dev/null 2>&1 || true
      done
    done
  fi
  if [ -n "$owned_pid" ]; then
    kill "$owned_pid" 2>/dev/null || true
    for ((attempt = 0; attempt < 10; attempt++)); do
      kill -0 "$owned_pid" 2>/dev/null || break
      sleep 1
    done
    kill -9 "$owned_pid" 2>/dev/null || true
    wait "$owned_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
if [ "$journey" = playback ]; then
  playback_run=$(uuidgen | tr '[:upper:]' '[:lower:]')
  playback_stage="$OUT/playback-stage-$playback_run"
  mkdir "$playback_stage"
  rm -f "$OUT/playback/manifest.json"
fi

: "${SCREENROOM_API_ENDPOINT:?Set the caller-owned backend endpoint}"
: "${SCREENROOM_USER_ID:?Set the explicit local user UUID}"
: "${SCREENROOM_BACKEND_DESCRIPTOR:?Set the caller-owned backend descriptor}"
[ "${SCREENROOM_BACKEND_ATTACH:-}" = 1 ] || fail 'SCREENROOM_BACKEND_ATTACH=1 is required'
[ "${SCREENROOM_E2E_API_ENDPOINT:-$SCREENROOM_API_ENDPOINT}" = "$SCREENROOM_API_ENDPOINT" ] || fail 'Endpoint aliases differ'
[[ "$SCREENROOM_API_ENDPOINT" =~ ^http://(127\.0\.0\.1|localhost):[0-9]+$ ]] || fail 'Use an explicit loopback HTTP origin with ADB reverse'
bash "$ROOT/../../scripts/backend.sh" wait
bash "$ROOT/../../scripts/backend.sh" status | tee "$OUT/backend-status.json"
instance=$(curl --connect-timeout 5 --max-time 15 --silent --show-error --fail "$SCREENROOM_API_ENDPOINT/__screenroom_instance")
printf '%s\n' "$instance" > "$OUT/backend-instance.json"
python3 - "$SCREENROOM_BACKEND_DESCRIPTOR" "$OUT/backend-instance.json" <<'PY'
import json, os, sys
descriptor, actual = (json.load(open(path)) for path in sys.argv[1:])
assert descriptor['instanceID'] == actual['instanceID']
assert descriptor['publicOrigin'] == actual['publicOrigin'] == os.environ['SCREENROOM_API_ENDPOINT']
assert not os.environ.get('SCREENROOM_BACKEND_INSTANCE_ID') or os.environ['SCREENROOM_BACKEND_INSTANCE_ID'] == actual['instanceID']
print('Android E2E backend instance=' + actual['instanceID'])
PY
api_port=${SCREENROOM_API_ENDPOINT##*:}
[[ "$api_port" =~ ^[0-9]+$ ]] || fail 'Endpoint must specify an explicit port'

tasks=()
for profile in "${profiles[@]}"; do
  rm -f "$OUT/$profile-instrumentation.txt"
  tasks+=(":$profile:assembleDebug" ":$profile:assembleDebugAndroidTest")
done

for profile in "${profiles[@]}"; do
  if [ "$profile" = mobile ]; then
    serial=${E2E_ANDROID_MOBILE_SERIAL:-emulator-5554}
    avd=${E2E_ANDROID_MOBILE_AVD:-screenroom_home_mobile}
  else
    serial=${E2E_ANDROID_TV_SERIAL:-emulator-5556}
    avd=${E2E_ANDROID_TV_AVD:-screenroom_home_tv}
  fi
  [[ "$serial" =~ ^emulator-[0-9]+$ ]] || fail "Not an emulator serial: $serial"
  port=${serial#emulator-}
  [[ "$port" =~ ^[1-9][0-9]{3}$ ]] || fail "Invalid emulator port: $port"
  (( port >= 5554 && port <= 5682 && port % 2 == 0 )) || fail "Invalid emulator port: $port"
  adb() { timeout 15 "$ADB" -s "$serial" "$@"; }

  devices=$(timeout 15 "$ADB" devices)
  if echo "$devices" | awk -v serial="$serial" '$1 == serial { found = 1 } END { exit !found }'; then
    echo "Android E2E: reuse $serial ($avd)"
  else
    timeout 15 "$EMULATOR" -list-avds | grep -Fxq "$avd" || fail "AVD not found: $avd"
    echo "Android E2E: boot $serial ($avd)"
    nohup "$EMULATOR" -avd "$avd" -port "$port" -no-window -no-audio \
      -no-boot-anim -gpu swiftshader -camera-back none \
      >"$OUT/$profile-emulator.log" 2>&1 &
    owned_pid=$!
  fi

  deadline=$((SECONDS + 180))
  until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do
    if [ -n "$owned_pid" ]; then
      kill -0 "$owned_pid" 2>/dev/null || fail "Emulator exited; inspect $OUT/$profile-emulator.log"
    fi
    (( SECONDS < deadline )) || fail "$serial boot exceeded 180 seconds"
    sleep 2
  done
  actual_avd=$(adb emu avd name | tr -d '\r' | sed '/^OK$/d')
  [ "$actual_avd" = "$avd" ] || fail "$serial uses AVD '$actual_avd', expected '$avd'"
  [ "$(adb shell getprop ro.kernel.qemu | tr -d '\r')" = 1 ] || fail "$serial is not an Android emulator"
  features=$(adb shell pm list features)
  if echo "$features" | grep -q 'feature:android.software.leanback'; then
    [ "$profile" = tv ] || fail "$serial is a TV emulator, not mobile"
  else
    [ "$profile" = mobile ] || fail "$serial is a mobile emulator, not TV"
  fi
  # A ready, verified emulator stays available for the next run.
  owned_pid=

  package=tv.screenroom
  if [[ $profile == tv ]]; then package=tv.screenroom.tv; fi
  verified_serials+=("$serial")
  verified_packages+=("$package")
  reverse=$(adb reverse --list)
   if ! echo "$reverse" | grep -Eq "tcp:$api_port tcp:$api_port$"; then
    echo "$reverse" | grep -q "tcp:$api_port " && fail 'Existing reverse maps endpoint port elsewhere'
    adb reverse "tcp:$api_port" "tcp:$api_port"
    owned_reverses+=("$serial")
  fi
  if adb shell pm path "$package" | grep -q '^package:'; then
    for flow in "${flows[@]}"; do
      adb shell run-as "$package" rm -f "files/screenshots/$flow.png"
    done
  fi
done

echo 'Android E2E: incremental APK build'
(cd "$ROOT" && timeout 600 ./gradlew --build-cache -Pandroid.builder.sdkDownload=false "${tasks[@]}") 2>&1 | tee "$OUT/build.log"

for ((index = 0; index < ${#profiles[@]}; index++)); do
  profile=${profiles[$index]}
  serial=${verified_serials[$index]}
  package=${verified_packages[$index]}
  test_class="org.screenroom.$profile.e2e.HomeE2ETest"
  timeout 120 "$ADB" -s "$serial" install -r -t "$ROOT/$profile/build/outputs/apk/debug/$profile-debug.apk"
  timeout 120 "$ADB" -s "$serial" install -r -t "$ROOT/$profile/build/outputs/apk/androidTest/debug/$profile-debug-androidTest.apk"
  adb shell input keyevent KEYCODE_WAKEUP
  adb shell wm dismiss-keyguard
  for flow in "${flows[@]}"; do
    adb shell run-as "$package" rm -f "files/screenshots/$flow.png"
  done
  for ((category_index = 0; category_index < ${#categories[@]}; category_index++)); do
    category=${categories[$category_index]}
    method=${methods[$category_index]}
    log="$OUT/$profile-$category-instrumentation.txt"
    echo "Android E2E: $profile category=$category method=$method"
    showcase_args=()
    if [ "$journey" = playback ]; then
      for selector in SCREENROOM_SHOWCASE_ITEM_ID SCREENROOM_SHOWCASE_TITLE SCREENROOM_SHOWCASE_YEAR SCREENROOM_BACKEND_INSTANCE_ID; do
        printf -v quoted_selector '%q' "${!selector}"
        showcase_args+=(-e "$selector" "$quoted_selector")
      done
      showcase_args+=(-e SCREENROOM_SHOWCASE_RUN "$playback_run")
    fi
    run_owned_instrumentation 300 "$serial" "$package" "$log" "${showcase_args[@]}" -e SCREENROOM_API_ENDPOINT "$SCREENROOM_API_ENDPOINT" -e SCREENROOM_USER_ID "$SCREENROOM_USER_ID" -e SCREENROOM_E2E_SETTINGS_NAMESPACE "$profile_prefix-$profile-$category" -e class \
      "$test_class#$method"
    cat "$log" >> "$OUT/$profile-instrumentation.txt"
    grep -Eq '^OK \(1 test\)' "$log" || fail "$profile $category E2E failed; no screenshot exported"
    if [ "$journey" = playback ]; then
      grep -Fxq 'INSTRUMENTATION_STATUS: test=playbackShowcase' "$log" || fail 'Playback method did not execute'
      grep -Fxq "INSTRUMENTATION_STATUS: class=$test_class" "$log" || fail 'Unexpected playback test class'
      ! grep -Eq '^INSTRUMENTATION_STATUS_CODE: -[34]' "$log" || fail 'Skipped playback test cannot export captures'
    fi
    if [ "$category" = regression ]; then
      run_owned_instrumentation 180 "$serial" "$package" "$OUT/$profile-correction-edges-instrumentation.txt" \
        -e SCREENROOM_API_ENDPOINT "$SCREENROOM_API_ENDPOINT" -e SCREENROOM_USER_ID "$SCREENROOM_USER_ID" \
        -e SCREENROOM_E2E_SETTINGS_NAMESPACE "$profile_prefix-$profile-edges" -e class "$test_class#correctionEdges"
      for phase in write read; do
        timeout 15 "$ADB" -s "$serial" shell am force-stop "$package"
        persistence_log="$OUT/$profile-settings-$phase-instrumentation.txt"
        run_owned_instrumentation 120 "$serial" "$package" "$persistence_log" \
          -e SCREENROOM_API_ENDPOINT "$SCREENROOM_API_ENDPOINT" -e SCREENROOM_USER_ID "$SCREENROOM_USER_ID" \
          -e SCREENROOM_E2E_SETTINGS_NAMESPACE "$profile_prefix-$profile-persistence" -e SCREENROOM_SETTINGS_PHASE "$phase" \
          -e class "$test_class#settingsPersistence"
        grep -Eq '^OK \(1 test\)' "$persistence_log" || fail "$profile process-persistence $phase failed"
      done
    fi
  done
  [ "$capture" = true ] || continue
  if [ "$journey" = playback ]; then
    slug=android
    [ "$profile" != tv ] || slug=android-tv
    mkdir "$playback_stage/$slug"
    timeout 30 "$ADB" -s "$serial" exec-out run-as "$package" cat "files/screenshots/playback/$playback_run/manifest.json" \
      > "$playback_stage/manifest.json"
    python3 "$ROOT/scripts/playback-showcase.py" paths "$playback_stage" "$slug" > "$playback_stage/paths.txt"
    while IFS= read -r image; do
      timeout 30 "$ADB" -s "$serial" exec-out run-as "$package" cat "files/screenshots/playback/$playback_run/${image##*/}" \
        > "$playback_stage/$image"
    done < "$playback_stage/paths.txt"
    mv "$playback_stage/manifest.json" "$playback_stage/$slug.json"
    rm "$playback_stage/paths.txt"
    continue
  fi
  for flow in "${flows[@]}"; do
    timeout 30 "$ADB" -s "$serial" exec-out run-as "$package" cat "files/screenshots/$flow.png" \
      >"$OUT/$profile-$flow.png.tmp"
    signature=$(od -An -tx1 -N8 "$OUT/$profile-$flow.png.tmp" | tr -d ' \n')
    [ "$signature" = 89504e470d0a1a0a ] || fail "$profile $flow screenshot is not a PNG"
    dimensions=$(file -b "$OUT/$profile-$flow.png.tmp")
    echo "$dimensions" | grep -Eq '^PNG image data, [1-9][0-9]* x [1-9][0-9]*,' \
      || fail "$profile $flow PNG dimensions are invalid"
  done
done

if [ "$journey" = playback ]; then
  slugs=()
  for profile in "${profiles[@]}"; do
    if [ "$profile" = tv ]; then slugs+=(android-tv); else slugs+=(android); fi
  done
  python3 "$ROOT/scripts/playback-showcase.py" promote "$playback_stage" "${slugs[@]}"
  echo "Android E2E: playback evidence and cleanup report in $OUT/playback/manifest.json (not media acceptance)"
  exit 0
fi

for profile in "${profiles[@]}"; do
  if [ "$capture" != true ]; then
    echo "Android E2E: passed $profile categories=${categories[*]}; no screenshots for non-happy categories"
    continue
  fi
  for flow in "${flows[@]}"; do
    mv "$OUT/$profile-$flow.png.tmp" "$OUT/$profile-$flow.png"
  done
  echo "Android E2E: passed $profile; five screenshots in $OUT"
done
