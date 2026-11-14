#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
# Each background command gets an owned process group, including its descendants.
set -m

case "${E2E_MACOS-0}" in
  0|1) ;;
  *) echo 'E2E_MACOS must be 0 or 1 (or unset)' >&2; exit 2 ;;
esac
apple_profiles=(ios ipad tvos)
flows=(home sidebar library search detail)
if [ "${E2E_MACOS-0}" = 1 ]; then apple_profiles+=(macos); fi

usage() {
  echo 'Usage: bash scripts/e2e.sh [--update-showcase] [--category happy-path|retry|regression|all] [apple|android|linux|web ...]'
}
update_showcase=false
families=
category=${E2E_CATEGORY-}
category_set=${E2E_CATEGORY+x}
while [ "$#" -gt 0 ]; do
  arg=$1
  shift
  case "$arg" in
    --update-showcase) update_showcase=true ;;
    --category)
      [ "$#" -gt 0 ] || { echo '--category requires a value.' >&2; exit 2; }
      category=$1
      category_set=x
      shift ;;
    -h|--help) usage; exit 0 ;;
    apple|android|linux|web)
      case " $families " in *" $arg "*) ;; *) families="${families:+$families }$arg" ;; esac
      ;;
    *)
      printf 'Unknown E2E family or option: %s\n' "$arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done
families=${families:-apple android linux web}
if [ "$category_set" != x ]; then
  category=all
  if "$update_showcase"; then category=happy-path; fi
fi
case "$category" in
  happy-path|retry|regression|all) ;;
  *) printf 'Invalid E2E category: "%s". Expected happy-path, retry, regression, or all.\n' "$category" >&2; exit 2 ;;
esac
if "$update_showcase"; then
  case "$category" in
    happy-path|all) ;;
    *) echo 'Showcase export requires category happy-path or all, which includes happy-path captures.' >&2; exit 2 ;;
  esac
fi
export E2E_CATEGORY=$category
command -v node >/dev/null || { echo 'Root E2E orchestration requires Node.js on PATH, including native-only selections.' >&2; exit 1; }
if [ -n "${SCREENROOM_API_ENDPOINT:-}" ] && [ -n "${SCREENROOM_E2E_API_ENDPOINT:-}" ] && [ "$SCREENROOM_API_ENDPOINT" != "$SCREENROOM_E2E_API_ENDPOINT" ]; then
  echo 'Backend endpoint aliases must match.' >&2
  exit 2
fi

stage=
backend_owned=0
root_lock=
mkdir -p build/e2e/runs
if ! mkdir build/e2e/owner.lock 2>/dev/null; then
  echo 'Another root E2E owner holds build/e2e/owner.lock; do not run concurrent roots.' >&2
  exit 1
fi
root_lock=build/e2e/owner.lock
trap 'rm -f "$root_lock/pid"; rmdir "$root_lock"' EXIT
echo "$$" >"$root_lock/pid"
run=$(mktemp -d "$PWD/build/e2e/runs/$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")
echo "E2E diagnostics: $run"
echo "E2E category: $category; families: $families"
pids=()
names=()
owners=()
started=()
export_targets=()
cleanup() {
  status=$?
  rollback_failed=0
  trap - EXIT INT TERM HUP
  node scripts/e2e-cleanup.mjs "${owners[@]}" || true
  for pid in "${pids[@]}"; do
    [ -n "$pid" ] || continue
    wait "$pid" 2>/dev/null || true
  done
  if [ "$backend_owned" = 1 ] && [ -f "$run/backend-owned" ]; then
    docker compose -p "${SCREENROOM_BACKEND_PROJECT:-screenroom-local}" logs --no-color >"$run/backend-containers.log" 2>&1 || true
    if ! bash scripts/backend.sh down --owner-file "$run/backend-owned" >"$run/backend-down.log" 2>&1; then
      echo "Owned backend teardown failed; see $run/backend-down.log" >&2
      [ "$status" -ne 0 ] || status=1
    fi
  fi
  if [ -n "$root_lock" ]; then rm -f "$root_lock/pid"; rmdir "$root_lock"; fi
  if [ "$status" -ne 0 ]; then
    for target in "${export_targets[@]}"; do
      if [ -f "$stage/original/$target" ]; then
        cp "$stage/original/$target" "website/static/showcase/$target" || rollback_failed=1
      else
        rm -f "website/static/showcase/$target"
      fi
    done
  fi
  if [ "$rollback_failed" = 1 ]; then
    echo "Showcase rollback failed; preserve backups at $stage/original" >&2
  else
    [ -z "$stage" ] || rm -rf "$stage"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
timeout_bin=$(command -v timeout || command -v gtimeout || true)
[ -n "$timeout_bin" ] || { echo 'E2E requires GNU timeout.' >&2; exit 1; }
launch() {
  local name=$1 limit=2400s; shift
  [ "$name" != backend ] || limit=6000s
  node scripts/e2e-group.mjs "$run/$name.status" "$timeout_bin" --foreground -k 20s "$limit" "$@" >"$run/$name.log" 2>&1 &
  pids+=("$!")
  owners+=("$!|$(ps -p "$!" -o lstart= 2>/dev/null || true)|anchor")
  names+=("$name")
  started+=("$SECONDS")
  echo "E2E started: $name (category $category; $run/$name.log)"
}
finish() {
  local index=$1 status=0
  while [ ! -f "$run/${names[$index]}.status" ]; do
    if ! kill -0 "${pids[$index]}" 2>/dev/null; then
      wait "${pids[$index]}" || status=$?
      [ "$status" -ne 0 ] || status=1
      break
    fi
    sleep 0.2
  done
  if [ -f "$run/${names[$index]}.status" ]; then status=$(cat "$run/${names[$index]}.status"); fi
  local elapsed=$((SECONDS - started[index]))
  printf 'category=%s\nexit=%s\nelapsed_seconds=%s\n' "$category" "$status" "$elapsed" >"$run/${names[$index]}.summary"
  printf 'E2E finished: %s (category %s; exit %s; elapsed %ss; %s/%s.log)\n' "${names[$index]}" "$category" "$status" "$elapsed" "$run" "${names[$index]}"
  return "$status"
}
prepare() {
  launch "$@"
  finish "$((${#pids[@]} - 1))"
}
if "$update_showcase"; then
  command -v file >/dev/null || { echo 'Showcase export requires file.' >&2; exit 1; }
  case " $families " in
    *' apple '*) command -v jq >/dev/null || { echo 'Showcase export requires jq.' >&2; exit 1; } ;;
  esac
  stage=$(mktemp -d "${TMPDIR:-/tmp}/screenroom-showcase.XXXXXX")
fi

case " $families " in
  *' apple '*)
    if [ "${E2E_APPLE_RUN_DIR+x}" = x ]; then
      apple_run=$E2E_APPLE_RUN_DIR
      case "$apple_run" in
        /*) ;;
        *) echo 'E2E_APPLE_RUN_DIR must be an absolute, new directory' >&2; exit 2 ;;
      esac
      if [ -e "$apple_run" ] || [ -L "$apple_run" ] || [ ! -d "$(dirname "$apple_run")" ]; then
        echo 'E2E_APPLE_RUN_DIR must be a new directory with an existing parent' >&2
        exit 2
      fi
    else
      mkdir -p apps/apple/build/e2e/runs
      apple_run=$(mktemp -d "$PWD/apps/apple/build/e2e/runs/showcase-XXXXXX")/run
    fi
    printf 'Persistent Apple E2E artifact path: %s\n' "$apple_run"
    prepare prepare-apple bash apps/apple/scripts/prepare-core.sh ;;
esac
case " $families " in *' android '*) prepare prepare-android bash apps/android/scripts/build-core.sh ;; esac
case " $families " in
  *' web '*)
    [ "$(pnpm --version)" = 12.18.0 ] || { echo 'E2E requires pnpm 12.18.0.' >&2; exit 1; }
    prepare prepare-web node apps/web/scripts/prepare-core.mjs ;;
esac
export SCREENROOM_CORE_PREPARED=1
export SCREENROOM_E2E_PROFILE_PREFIX="run-$(basename "$run")"

export SCREENROOM_API_ENDPOINT=${SCREENROOM_API_ENDPOINT:-${SCREENROOM_E2E_API_ENDPOINT:-http://127.0.0.1:${SCREENROOM_BACKEND_PORT:-50051}}}
[ -z "${SCREENROOM_E2E_API_ENDPOINT:-}" ] || [ "$SCREENROOM_API_ENDPOINT" = "$SCREENROOM_E2E_API_ENDPOINT" ] || { echo 'Backend endpoint aliases must match.' >&2; exit 2; }
export SCREENROOM_E2E_API_ENDPOINT=$SCREENROOM_API_ENDPOINT
case "${SCREENROOM_BACKEND_ATTACH:-0}" in
  1) prepare backend bash scripts/backend.sh wait ;;
  0)
    if bash scripts/backend.sh status >"$run/backend-existing.json" 2>/dev/null; then
      echo 'A backend is already running. Set SCREENROOM_BACKEND_ATTACH=1 to attach without teardown.' >&2
      exit 2
    fi
    backend_owned=1
    prepare backend bash scripts/backend.sh up --owner-file "$run/backend-owned" ;;
  *) echo 'SCREENROOM_BACKEND_ATTACH must be 0 or 1.' >&2; exit 2 ;;
esac
export SCREENROOM_BACKEND_DESCRIPTOR="$PWD/build/backend-runtime/${SCREENROOM_BACKEND_PROJECT:-screenroom-local}/descriptor.json"
actual_user=$(node -e 'const d=JSON.parse(require("fs").readFileSync(process.env.SCREENROOM_BACKEND_DESCRIPTOR));if(d.ready!==true||!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(d.userID))throw Error("Installation user selector missing");console.log(d.userID)')
[ -z "${SCREENROOM_USER_ID:-}" ] || [ "$SCREENROOM_USER_ID" = "$actual_user" ] || { echo 'User selector must match the installation descriptor.' >&2; exit 2; }
export SCREENROOM_USER_ID=$actual_user
cp "$SCREENROOM_BACKEND_DESCRIPTOR" "$run/backend-instance.json"
export SCREENROOM_BACKEND_INSTANCE_ID=$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.env.SCREENROOM_BACKEND_DESCRIPTOR)).instanceID)')
for family in $families; do cp "$SCREENROOM_BACKEND_DESCRIPTOR" "$run/$family-instance.json"; done

# Explicit Mac opt-in means the user is ready now, never after an unattended batch.
case " $families " in
  *' apple '*)
    if [ "${E2E_MACOS-0}" = 1 ]; then
      mac_run="$run/apple-macos"
      prepare macos env E2E_APPLE_RUN_DIR="$mac_run" bash apps/apple/scripts/e2e.sh macos
    fi ;;
esac
first_group=${#pids[@]}
for family in $families; do
  case "$family" in
    apple) launch apple env E2E_MACOS=0 E2E_APPLE_RUN_DIR="$apple_run" bash apps/apple/scripts/e2e.sh ;;
    android) launch android bash apps/android/scripts/e2e.sh ;;
    linux) launch linux bash apps/linux/tests/e2e/run.sh ;;
    web) launch web pnpm --filter @screenroom/web test:e2e ;;
  esac
done
failed=0
remaining=$((${#pids[@]} - first_group))
completed=()
while [ "$remaining" -gt 0 ]; do
  for ((index=first_group; index<${#pids[@]}; index++)); do
    [ "${completed[$index]-}" != 1 ] || continue
    if [ -f "$run/${names[$index]}.status" ] || ! kill -0 "${pids[$index]}" 2>/dev/null; then
      finish "$index" || failed=1
      completed[$index]=1
      remaining=$((remaining - 1))
    fi
  done
  if [ "$remaining" -gt 0 ]; then sleep 0.2; fi
done
[ "$failed" = 0 ] || exit 1

"$update_showcase" || exit 0

capture() {
  local source=$1 target=$2 signature
  [ -f "$source" ] || { printf 'Missing showcase capture: %s\n' "$source" >&2; exit 1; }
  signature=$(od -An -tx1 -N8 "$source" | tr -d ' \n')
  if [ "$signature" != 89504e470d0a1a0a ] || [ "$(file -b --mime-type "$source")" != image/png ]; then
    printf 'Invalid PNG showcase capture: %s\n' "$source" >&2
    exit 1
  fi
  mkdir -p "$stage/export/$(dirname "$target")"
  cp "$source" "$stage/export/$target"
  printf 'Showcase capture: %s -> website/static/showcase/%s\n' "$source" "$target"
}

for family in $families; do
  case "$family" in
    apple)
      for profile in "${apple_profiles[@]}"; do
        screenshots="$apple_run/$profile/screenshots"
        [ "$profile" != macos ] || screenshots="$mac_run/macos/screenshots"
        for flow in "${flows[@]}"; do
          filename=$(jq -er --arg flow "$flow" '
            [.[].attachments[]
             | select(.isAssociatedWithFailure == false)
             | select(.suggestedHumanReadableName | test("^" + $flow + "-success(_.*)?\\.png$"))
             | .exportedFileName]
            | if length == 1 and (.[0] | test("^[^/\\\\]+\\.png$")) then .[0]
             else error("Expected exactly one named " + $flow + "-success PNG attachment") end
          ' "$screenshots/manifest.json")
          slug=$profile
          [ "$profile" != ipad ] || slug=ipados
          capture "$screenshots/$filename" "$slug/$flow.png"
        done
      done
      ;;
    android)
      for flow in "${flows[@]}"; do
        capture "apps/android/build/e2e/mobile-$flow.png" "android/$flow.png"
        capture "apps/android/build/e2e/tv-$flow.png" "android-tv/$flow.png"
      done
      ;;
    linux)
      for flow in "${flows[@]}"; do
        capture "apps/linux/build/e2e/$flow.png" "linux/$flow.png"
      done
      ;;
    web)
      for flow in "${flows[@]}"; do
        for profile in desktop mobile; do
          target="web/$profile-$flow.png"
          [ "$flow" != home ] || target="web/$profile.png"
          capture "apps/web/e2e/build/screenshots/$flow-$profile.png" "$target"
        done
      done
      ;;
  esac
done

# Nothing under the website is written until every selected capture is staged.
for source in "$stage"/export/*/*.png; do
  target=${source#"$stage/export/"}
  if [ -f "website/static/showcase/$target" ]; then
    mkdir -p "$stage/original/$(dirname "$target")"
    cp "website/static/showcase/$target" "$stage/original/$target"
  fi
done
for source in "$stage"/export/*/*.png; do
  target=${source#"$stage/export/"}
  export_targets+=("$target")
  mkdir -p "website/static/showcase/$(dirname "$target")"
  cp "$source" "website/static/showcase/$target"
done
echo 'Showcase screenshots updated.'
