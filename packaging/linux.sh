#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
target=$1 version=$2 build=$3 work=$4 artifact=$5
[[ $(uname -s) == Linux ]] || die 'use a native Linux build host'
source /etc/os-release
case "$target" in
    linux-deb-*)
        arch=${target#linux-deb-}
        [[ $ID == debian && $VERSION_ID == 13 ]] || die 'use Debian 13 (trixie)'
        need dpkg dpkg-deb dpkg-shlibdeps
        [[ $(dpkg --print-architecture) == "$arch" ]] || die "use Debian $arch"
        if [[ $arch == amd64 ]]; then machine=x86_64; else machine=aarch64; fi ;;
    linux-arch-amd64)
        [[ $ID == arch && $EUID != 0 ]] || die 'use Arch Linux as an unprivileged user'
        need makepkg bsdtar
        machine=x86_64 ;;
    *) die "unknown Linux target: $target" ;;
esac
need cargo rustc make desktop-file-validate readelf
triple="$machine-unknown-linux-gnu"
[[ $(uname -m) == "$machine" && $(rustc -vV | sed -n 's/^host: //p') == "$triple" ]] ||
    die "use a native $triple Rust toolchain and host"

cargo build --manifest-path "$root/apps/linux/Cargo.toml" --locked --release \
    --target "$triple" --target-dir "$root/target/native-linux" --bin screenroom-linux
stage="$work/stage"
install -Dm755 "$root/target/native-linux/$triple/release/screenroom-linux" "$stage/usr/bin/screenroom-linux"
make -C "$root/apps/linux/data" install "DESTDIR=$stage" PREFIX=/usr
desktop-file-validate "$stage/usr/share/applications/org.screenroom.linux.desktop"
elf=$(LC_ALL=C readelf -h "$stage/usr/bin/screenroom-linux")
case "$machine:$elf" in
    x86_64:*'Advanced Micro Devices X86-64'*|aarch64:*AArch64*) ;;
    *) die "unexpected ELF architecture for $target" ;;
esac

cd "$work"
if [[ $target == linux-deb-* ]]; then
    mkdir debian "$stage/DEBIAN"
    printf 'Source: screenroom\n\nPackage: screenroom\nArchitecture: any\n' >debian/control
    deps=$(dpkg-shlibdeps -O -e stage/usr/bin/screenroom-linux)
    [[ $deps == shlibs:Depends=?* && $deps != *$'\n'* ]] || die 'could not derive shared library dependencies'
    cat >"$stage/DEBIAN/control" <<EOF
Package: screenroom
Version: $version-$build
Architecture: $arch
Maintainer: Screenroom
Section: video
Priority: optional
Depends: ${deps#shlibs:Depends=}, gstreamer1.0-gtk4, gstreamer1.0-plugins-base, gstreamer1.0-plugins-good, gstreamer1.0-plugins-bad, gstreamer1.0-libav, gstreamer1.0-x
Description: Native Screenroom catalog and playback client
EOF
    dpkg-deb --root-owner-group --build "$stage" "$artifact"
    [[ $(dpkg-deb -f "$artifact" Package) == screenroom &&
       $(dpkg-deb -f "$artifact" Architecture) == "$arch" &&
       $(dpkg-deb -f "$artifact" Version) == "$version-$build" ]] || die 'unexpected Debian metadata'
else
    cp "$root/packaging/arch/PKGBUILD" .
    SCREENROOM_VERSION="$version" SCREENROOM_BUILD="$build" SCREENROOM_STAGE="$stage" \
        BUILDDIR="$work" PKGDEST="$work" SRCDEST="$work" LOGDEST="$work" PKGEXT=.pkg.tar.zst \
        makepkg --nocolor --nosign
    info=$(bsdtar -xOf "$artifact" .PKGINFO)
    for field in 'pkgname = screenroom' "pkgver = $version-$build" 'arch = x86_64'; do
        grep -Fxq "$field" <<<"$info" || die "missing Arch metadata: $field"
    done
fi
