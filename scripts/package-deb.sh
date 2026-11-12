#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
version=${1:?Usage: sh scripts/package-deb.sh VERSION}
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
test "$(dpkg --print-architecture)" = amd64
test -x target/release/branchline
mkdir -p target
work=$(mktemp -d "$(pwd)/target/deb-stage.XXXXXX")
root="$work/package"
mkdir -p "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" \
    "$root/usr/share/doc/bmm" "$work/debian" dist
install -m 755 target/release/branchline "$root/usr/bin/bmm"
install -m 644 packaging/bmm.desktop "$root/usr/share/applications/bmm.desktop"
install -m 644 packaging/INSTALL-Linux.txt "$root/usr/share/doc/bmm/README"
desktop-file-validate "$root/usr/share/applications/bmm.desktop"
cat > "$work/debian/control" <<EOF
Source: bmm
Section: utils
Priority: optional
Maintainer: BMM contributors <noreply@github.com>

Package: bmm
Architecture: amd64
Description: Local-first mind mapper
EOF
binary="$(pwd)/target/release/branchline"
depends=$(cd "$work" && dpkg-shlibdeps -O -e"$binary")
depends=${depends#shlibs:Depends=}
test -n "$depends"
size=$(du -sk "$root/usr" | cut -f1)
cat > "$root/DEBIAN/control" <<EOF
Package: bmm
Version: $version
Architecture: amd64
Section: utils
Priority: optional
Maintainer: BMM contributors <noreply@github.com>
Installed-Size: $size
Depends: $depends, libxkbcommon0, libxcb1, libxcb-render0, libxcb-shape0, libxcb-xfixes0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libegl1, libgl1, xdg-utils
Homepage: https://github.com/bojke2000/bmm
Description: Local-first keyboard-driven mind mapper
 Create and organize ideas on a two-sided canvas with notes, folding,
 branch focus, undo, multiple maps and local recovery.
EOF
dpkg-deb --root-owner-group --build "$root" "dist/bmm_${version}_amd64.deb"
dpkg-deb --info "dist/bmm_${version}_amd64.deb"
dpkg-deb --contents "dist/bmm_${version}_amd64.deb"
