#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
cd "$root"
[[ $# == 3 ]] || die 'usage: just package TARGET VERSION BUILD'
target=$1 version=$2 build=$3
release_inputs "$version" "$build"
artifact_metadata "$target" "$version" "$build"
need git jq shasum

if [[ ${SCREENROOM_PACKAGE_LOCAL:-0} == 1 ]]; then
    commit=local
    release_dir="$root/target/packages/local/v$version-$build"
else
    [[ -z $(git status --porcelain --untracked-files=normal) ]] ||
        die 'a publishable package needs a clean tree (SCREENROOM_PACKAGE_LOCAL=1 for a local check)'
    commit=$(git rev-parse HEAD)
    release_dir="$root/target/packages/v$version-$build"
fi
dest="$release_dir/$target"
[[ ! -e $dest ]] || die "package already exists: $dest"
mkdir -p "$release_dir"
work=$(mktemp -d "$release_dir/.build-$target.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT

case "$target" in
    linux-*) bash packaging/linux.sh "$target" "$version" "$build" "$work" "$artifact" ;;
    macos|ios|tvos) bash packaging/apple.sh "$target" package "$version" "$build" "$work" "$artifact" ;;
    android|android-tv) bash packaging/android.sh "$target" "$version" "$build" "$work/$artifact" ;;
esac

file="$work/$artifact"
[[ -s $file ]] || die "missing artifact: $artifact"
if [[ $commit != local ]]; then
    [[ -z $(git status --porcelain --untracked-files=normal) && $(git rev-parse HEAD) == "$commit" ]] ||
        die 'source changed during the build'
fi
jq -n --arg app screenroom --arg target "$target" --arg architecture "$architecture" \
    --arg version "$version" --argjson build "$build" --arg commit "$commit" \
    --arg signature "$signature" --arg file "$artifact" \
    --argjson size "$(size "$file")" --arg sha256 "$(sha256 "$file")" \
    '{$app,$target,$architecture,$version,$build,$commit,$signature,$file,$size,$sha256}' >"$work/manifest.json"
mkdir "$dest"
mv -- "$file" "$work/manifest.json" "$dest/"
printf '%s\n' "$dest"
