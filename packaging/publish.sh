#!/usr/bin/env bash
set +x
set -euo pipefail
source "$(dirname "$0")/common.sh"
[[ $# -ge 1 && $# -le 2 ]] || die 'usage: just publish RELEASE_DIR [--dry-run]'
release_dir=$1 mode=${2:-}
[[ -z $mode || $mode == --dry-run ]] || die 'only --dry-run is supported'
if [[ -z $mode ]]; then
    unset token
    token=${FORGEJO_ACCESS_TOKEN:-}
fi
unset FORGEJO_ACCESS_TOKEN
base=${SCREENROOM_PACKAGE_URL:-}
[[ $base =~ ^https://[a-zA-Z0-9.-]+(:[0-9]+)?/api/packages/[a-zA-Z0-9_-]+/generic/[a-zA-Z0-9_-]+$ ]] ||
    die 'set SCREENROOM_PACKAGE_URL to https://HOST/api/packages/OWNER/generic/PACKAGE'
need jq shasum

while [[ $release_dir == */ && $release_dir != / ]]; do release_dir=${release_dir%/}; done
[[ -d $release_dir && ! -L $release_dir ]] || die 'use a release directory, not a symlink'
first_dir="$release_dir/${targets[0]}"
first="$first_dir/manifest.json"
[[ -d $first_dir && ! -L $first_dir && -f $first && ! -L $first ]] || die "missing manifest: $first"
identity=$(jq -ec '{version,build,commit}' "$first")
version=$(jq -er '.version | strings' <<<"$identity")
build=$(jq -er '.build | numbers' <<<"$identity")
commit=$(jq -er '.commit | strings' <<<"$identity")
release_inputs "$version" "$build"
[[ $commit =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || die 'release requires packages from a clean Git commit'
destination="$base/v$version-$build"
files=() manifests=() digests=()
shopt -s nullglob dotglob
entries=("$release_dir"/*)
[[ ${#entries[@]} == "${#targets[@]}" ]] || die 'release directory must contain exactly the eight target directories'
for target in "${targets[@]}"; do
    artifact_metadata "$target" "$version" "$build"
    dir="$release_dir/$target"
    file="$dir/$artifact" manifest="$dir/manifest.json"
    entries=("$dir"/*)
    [[ -d $dir && ! -L $dir && ${#entries[@]} == 2 &&
       -s $file && ! -L $file && -f $manifest && ! -L $manifest ]] ||
        die "expected just the package and manifest for $target"
    digest=$(sha256 "$file")
    validated=$(jq -ecs --arg target "$target" --arg architecture "$architecture" \
        --arg version "$version" --argjson build "$build" --arg commit "$commit" \
        --arg signature "$signature" --arg file "$artifact" \
        --argjson size "$(size "$file")" --arg sha256 "$digest" \
        'select(. == [{app:"screenroom",$target,$architecture,$version,$build,$commit,$signature,$file,$size,$sha256}]) | .[0]' \
        "$manifest") || die "manifest or checksum mismatch for $target"
    files+=("$file")
    manifests+=("$validated")
    digests+=("$digest")
done

if [[ $mode == --dry-run ]]; then
    for file in "${files[@]}"; do printf '%s -> %s/%s\n' "$file" "$destination" "${file##*/}"; done
    printf 'release.json -> %s/release.json (last)\n' "$destination"
    exit 0
fi
[[ $token =~ ^[A-Za-z0-9_-]+$ ]] || die 'set FORGEJO_ACCESS_TOKEN to a Forgejo package token'
need curl
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
printf '%s\n' "${manifests[@]}" | jq -s --arg version "$version" --argjson build "$build" --arg commit "$commit" \
    '{app:"screenroom",$version,$build,$commit,artifacts:.}' >"$work/release.json"

request() {
    printf 'Authorization: token %s\n' "$token" |
        curl --disable --silent --proto '=https' --connect-timeout 15 --max-time 300 \
            --header @- --write-out '%{http_code}' "$@"
}

publish_file() {
    local file=$1 digest=$2 name=${1##*/} url="$destination/${1##*/}" code
    code=$(request --output "$work/remote" -- "$url") || die "could not inspect remote $name"
    case "$code" in
        200) ;;
        404)
            [[ $(sha256 "$file") == "$digest" ]] || die "local file changed: $name"
            request --output /dev/null --upload-file "$file" -- "$url" >/dev/null || true
            code=$(request --output "$work/remote" -- "$url") ||
                die "upload outcome unknown for $name; check storage before retry"
            [[ $code == 200 ]] || die "could not confirm upload for $name (read-back HTTP $code)"
            ;;
        *) die "could not inspect remote $name (HTTP $code)" ;;
    esac
    [[ $digest == "$(sha256 "$work/remote")" ]] ||
        die "remote conflict for $name; no overwrite attempted"
    printf '%s\n' "$url"
}

for i in "${!files[@]}"; do publish_file "${files[i]}" "${digests[i]}"; done
publish_file "$work/release.json" "$(sha256 "$work/release.json")"
