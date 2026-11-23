#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/../common.sh"
need jq shasum
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/bin" "$work/remote" "$work/release"
export PATH="$work/bin:$PATH" RECORD="$work/requests" REMOTE="$work/remote"
export SCREENROOM_PACKAGE_URL=https://packages.invalid/api/packages/test/generic/screenroom
unset FORGEJO_ACCESS_TOKEN

cat >"$work/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ ! ${FORGEJO_ACCESS_TOKEN+x} && ! ${token+x} && $* != *fixture-token* ]]
IFS= read -r header
[[ $header == 'Authorization: token fixture-token' ]]
output= upload=
while [[ $# -gt 1 ]]; do
    case "$1" in
        --output) output=$2; shift ;;
        --upload-file) upload=$2; shift ;;
    esac
    shift
done
[[ $1 == "$SCREENROOM_PACKAGE_URL/v1.2.3-4/"* ]]
name=${1##*/}
if [[ -n $upload ]]; then
    printf 'PUT %s\n' "$name" >>"$RECORD"
    [[ ! -e $REMOTE/$name ]] || exit 1
    if [[ $name == "${FAIL_BEFORE:-}" ]]; then exit 28; fi
    cp "$upload" "$REMOTE/$name"
    if [[ $name == "${FAIL_AFTER:-}" ]]; then exit 28; fi
    printf 201
elif [[ -f $REMOTE/$name ]]; then
    cp "$REMOTE/$name" "$output"
    printf 200
else
    if [[ $name == "${CHANGE_BEFORE:-}" ]]; then
        printf 'changed after validation\n' >>"$CHANGE_FILE"
    fi
    : >"$output"
    printf 404
fi
EOF
chmod +x "$work/bin/curl"

publish() { bash "$root/packaging/publish.sh" "$work/release" "$@"; }
expect_failure() {
    if "$@" >"$work/error" 2>&1; then die "unexpected success: $*"; fi
}

commit=abcdabcdabcdabcdabcdabcdabcdabcdabcdabcd
for target in "${targets[@]}"; do
    artifact_metadata "$target" 1.2.3 4
    dir="$work/release/$target"
    mkdir "$dir"
    printf 'fixture for %s\n' "$target" >"$dir/$artifact"
    jq -n --arg target "$target" --arg architecture "$architecture" --arg signature "$signature" \
        --arg file "$artifact" --arg commit "$commit" --argjson size "$(size "$dir/$artifact")" \
        --arg sha256 "$(sha256 "$dir/$artifact")" \
        '{app:"screenroom",$target,$architecture,version:"1.2.3",build:4,$commit,$signature,$file,$size,$sha256}' \
        >"$dir/manifest.json"
done

plan=$(publish --dry-run)
[[ $plan == *'release.json (last)' && ! -e $RECORD ]]
ln -s "$work/release" "$work/linked-release"
expect_failure bash "$root/packaging/publish.sh" "$work/linked-release/" --dry-run
grep -q 'not a symlink' "$work/error"
expect_failure publish
grep -q FORGEJO_ACCESS_TOKEN "$work/error"
artifact_metadata android 1.2.3 4
printf 'changed\n' >>"$work/release/android/$artifact"
expect_failure publish --dry-run
grep -q 'checksum mismatch for android' "$work/error"
printf 'fixture for android\n' >"$work/release/android/$artifact"
mv "$work/release/tvos" "$work/tvos"
expect_failure publish --dry-run
mv "$work/tvos" "$work/release/tvos"

export FORGEJO_ACCESS_TOKEN=fixture-token
export CHANGE_BEFORE=$artifact CHANGE_FILE="$work/release/android/$artifact"
expect_failure publish
grep -q 'local file changed' "$work/error"
[[ ! -e $REMOTE/$artifact && ! -e $REMOTE/release.json ]]
unset CHANGE_BEFORE CHANGE_FILE
printf 'fixture for android\n' >"$work/release/android/$artifact"
rm "$RECORD"
rm -f "$REMOTE"/*
export FAIL_BEFORE=$artifact
expect_failure publish
[[ ! -e $REMOTE/release.json ]]
unset FAIL_BEFORE
export FAIL_AFTER=$artifact
publish >"$work/published"
unset FAIL_AFTER
[[ $(tail -n 1 "$RECORD") == 'PUT release.json' ]]
[[ $(wc -l <"$RECORD" | tr -d '[:space:]') == 10 ]]
jq -e '.artifacts | length == 8' "$REMOTE/release.json" >/dev/null
cp "$RECORD" "$work/last-record"
publish >/dev/null
cmp "$RECORD" "$work/last-record"
printf 'remote conflict\n' >"$REMOTE/$artifact"
expect_failure publish
grep -q 'remote conflict' "$work/error"
cmp "$RECORD" "$work/last-record"
! grep -q fixture-token "$work/error"
expect_failure bash "$root/packaging/package.sh" unknown 1.2.3 4
expect_failure bash "$root/packaging/package.sh" macos 1.2.3 0
expect_failure bash "$root/packaging/build.sh" build unknown
printf 'release tests passed\n'
