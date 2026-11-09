#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
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

# An unset category must get past validation; the fixture has no SDK or devices.
if env -u E2E_CATEGORY JAVA_HOME="$fixture/missing-jdk" ANDROID_HOME="$fixture/missing-sdk" /bin/bash "$fixture/scripts/e2e.sh" > "$fixture/unset.log" 2>&1; then
  echo 'Unset-category fixture unexpectedly reached instrumentation' >&2
  exit 1
fi
grep -Fq 'JDK 17 not found' "$fixture/unset.log"
! grep -Fq 'Invalid E2E_CATEGORY' "$fixture/unset.log"
echo 'Unset category retains normal all default and reaches environment validation'
