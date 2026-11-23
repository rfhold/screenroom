#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
for profile in mobile tv; do
  app_id=tv.screenroom
  if [[ $profile == tv ]]; then app_id=tv.screenroom.tv; fi
  grep -Fq "applicationId = \"$app_id\"" "$ROOT/$profile/build.gradle.kts"
  namespace="org.screenroom.$profile.e2e"
  source="$ROOT/$profile/src/androidTest/kotlin/${namespace//./\/}/HomeE2ETest.kt"
  grep -Fxq "package $namespace" "$source"
  grep -Fq 'class HomeE2ETest' "$source"
done
grep -Fq 'test_class="org.screenroom.$profile.e2e.HomeE2ETest"' "$ROOT/scripts/e2e.sh"
! grep -Fq '$package.e2e.' "$ROOT/scripts/e2e.sh" || exit 1
mkdir -p "$ROOT/build"
fixture=$(mktemp -d "$ROOT/build/category-test.XXXXXX")
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/build/e2e" "$fixture/bin"
cp "$ROOT/scripts/e2e.sh" "$fixture/scripts/e2e.sh"
cmp "$ROOT/scripts/e2e.sh" "$fixture/scripts/e2e.sh"
for profile in mobile tv; do
  for flow in home sidebar library search detail; do
    printf 'sentinel:%s:%s\n' "$profile" "$flow" > "$fixture/build/e2e/$profile-$flow.png"
  done
done
before=$(shasum -a 256 "$fixture"/build/e2e/*.png)
export TEST_COMMAND_MARKER="$fixture/commands"
for command in dirname mkdir rm uuidgen timeout adb; do
  cat > "$fixture/bin/$command" <<'STUB'
#!/bin/bash
printf '%s\n' "$0" >> "$TEST_COMMAND_MARKER"
exit 99
STUB
  chmod +x "$fixture/bin/$command"
done
for category in unknown ''; do
  if E2E_CATEGORY="$category" PATH="$fixture/bin:$PATH" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/rejection.log" 2>&1; then
    echo "Category '$category' unexpectedly succeeded" >&2
    exit 1
  else
    result=$?
    [ "$result" -eq 2 ]
  fi
  grep -Fq 'Invalid E2E_CATEGORY' "$fixture/rejection.log"
  [ ! -e "$TEST_COMMAND_MARKER" ]
  [ "$before" = "$(shasum -a 256 "$fixture"/build/e2e/*.png)" ]
  echo "Rejected category '$category': exit 2, no cleanup/preparation/device command, all ten sentinel SHA-256 hashes unchanged"
done
echo "$before"

for journey in unknown ''; do
  if E2E_CATEGORY=happy-path SCREENROOM_E2E_JOURNEY="$journey" PATH="$fixture/bin:$PATH" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/rejection.log" 2>&1; then exit 1; else [ "$?" -eq 2 ]; fi
  grep -Fq 'Invalid SCREENROOM_E2E_JOURNEY' "$fixture/rejection.log"
  [ ! -e "$TEST_COMMAND_MARKER" ]
  [ "$before" = "$(shasum -a 256 "$fixture"/build/e2e/*.png)" ]
done
for category in all retry regression; do
  if E2E_CATEGORY="$category" SCREENROOM_E2E_JOURNEY=playback PATH="$fixture/bin:$PATH" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/rejection.log" 2>&1; then exit 1; else [ "$?" -eq 2 ]; fi
  grep -Fq 'Playback journey requires E2E_CATEGORY=happy-path' "$fixture/rejection.log"
  [ ! -e "$TEST_COMMAND_MARKER" ]
  [ "$before" = "$(shasum -a 256 "$fixture"/build/e2e/*.png)" ]
done
if E2E_CATEGORY=happy-path SCREENROOM_E2E_JOURNEY=playback SCREENROOM_SHOWCASE_ITEM_ID='' PATH="$fixture/bin:$PATH" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/rejection.log" 2>&1; then exit 1; else [ "$?" -eq 2 ]; fi
grep -Fq 'Playback journey requires SCREENROOM_SHOWCASE_ITEM_ID' "$fixture/rejection.log"
[ ! -e "$TEST_COMMAND_MARKER" ]
for rejection in instance endpoint attach project; do
  instance=test-instance endpoint=http://127.0.0.1:50052 attach=1 project=screenroom-jellyfin-local
  case "$rejection" in
    instance) instance='' ;;
    endpoint) endpoint=http://127.0.0.1:50051 ;;
    attach) attach=0 ;;
    project) project=prototype ;;
  esac
  if E2E_CATEGORY=happy-path SCREENROOM_E2E_JOURNEY=playback SCREENROOM_SHOWCASE_ITEM_ID=explicit-item \
    SCREENROOM_SHOWCASE_TITLE=Nosferatu SCREENROOM_SHOWCASE_YEAR=1922 SCREENROOM_BACKEND_INSTANCE_ID="$instance" \
    SCREENROOM_API_ENDPOINT="$endpoint" SCREENROOM_BACKEND_ATTACH="$attach" SCREENROOM_BACKEND_PROJECT="$project" \
    PATH="$fixture/bin:$PATH" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/rejection.log" 2>&1; then exit 1; else [ "$?" -eq 2 ]; fi
  [ ! -e "$TEST_COMMAND_MARKER" ]
  [ "$before" = "$(shasum -a 256 "$fixture"/build/e2e/*.png)" ]
done
echo 'Playback rejects invalid journeys, incompatible categories and missing selectors before any command or cleanup'

# An unset category must get past validation; the fixture has no SDK or devices.
if env -u E2E_CATEGORY JAVA_HOME="$fixture/missing-jdk" ANDROID_HOME="$fixture/missing-sdk" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/unset.log" 2>&1; then
  echo 'Unset-category fixture unexpectedly reached instrumentation' >&2
  exit 1
fi
grep -Fq 'JDK 17 not found' "$fixture/unset.log"
! grep -Fq 'Invalid E2E_CATEGORY' "$fixture/unset.log"
echo 'Unset category retains normal all default and reaches environment validation'
