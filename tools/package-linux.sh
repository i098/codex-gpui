#!/usr/bin/env bash
# Build the release binary and package it as a .deb and an AppImage in OUT (default dist/).
# Needs dpkg-deb, python3 and appimagetool (APPIMAGETOOL overrides the path); skip one with
# FORMATS=deb or FORMATS=appimage. The .desktop file declares `x-scheme-handler/codex`, and
# `Exec=codex-gpui %U` hands the clicked codex:// URL to the app as argv.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(tools/version.sh)
out=$(realpath -m "${1:-dist}")
formats=${FORMATS:-deb appimage}
arch=$(uname -m)
case $arch in x86_64) deb_arch=amd64 ;; aarch64) deb_arch=arm64 ;; *) echo "unsupported arch $arch" >&2; exit 2 ;; esac
cargo build --locked --release -p app
binary=${CARGO_TARGET_DIR:-target}/release/codex-gpui
mkdir -p "$out"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT

# One tree serves both formats: usr/bin, usr/share/applications, usr/share/icons.
root=$stage/root
install -Dm755 "$binary" "$root/usr/bin/codex-gpui"
install -Dm644 /dev/stdin "$root/usr/share/applications/codex-gpui.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Codex GPUI
Comment=Native Codex desktop app
Exec=codex-gpui %U
Icon=codex-gpui
Terminal=false
Categories=Development;
MimeType=x-scheme-handler/codex;
StartupWMClass=codex-gpui
DESKTOP
# The 512x512 PNG inside the macOS icon (icns type ic09) is the Linux icon.
icon=$root/usr/share/icons/hicolor/512x512/apps/codex-gpui.png
mkdir -p "$(dirname "$icon")"
python3 - assets/app/AppIcon.icns "$icon" <<'PY'
import struct, sys
data = open(sys.argv[1], 'rb').read()
i = 8
while i < len(data):
    kind, size = data[i:i + 4], struct.unpack('>I', data[i + 4:i + 8])[0]
    if kind == b'ic09':
        open(sys.argv[2], 'wb').write(data[i + 8:i + size])
        break
    i += size
else:
    sys.exit('AppIcon.icns has no 512x512 PNG')
PY
mkdir -p "$root/usr/share/doc/codex-gpui"
python3 tools/third-party-notices.py "$root/usr/share/doc/codex-gpui/THIRD_PARTY_NOTICES.txt"

for format in $formats; do
    case $format in
    deb)
        deb=$stage/deb
        cp -a "$root" "$deb"
        mkdir -p "$deb/DEBIAN"
        cat >"$deb/DEBIAN/control" <<CONTROL
Package: codex-gpui
Version: $version
Section: devel
Priority: optional
Architecture: $deb_arch
Maintainer: Codex GPUI <noreply@users.noreply.github.com>
Depends: libc6, libgcc-s1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1, libfontconfig1, libasound2 | libasound2t64
Description: Native Codex desktop app
 Native GPUI client for Codex. Registers the codex:// URL scheme.
CONTROL
        fakeroot dpkg-deb --build "$deb" "$out/codex-gpui-$version-linux-$deb_arch.deb" >/dev/null
        ;;
    appimage)
        appdir=$stage/Codex_GPUI.AppDir
        cp -a "$root" "$appdir"
        ln -s usr/share/applications/codex-gpui.desktop "$appdir/codex-gpui.desktop"
        ln -s usr/share/icons/hicolor/512x512/apps/codex-gpui.png "$appdir/codex-gpui.png"
        printf '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/codex-gpui" "$@"\n' >"$appdir/AppRun"
        chmod +x "$appdir/AppRun"
        ARCH=$arch APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL:-appimagetool}" --no-appstream "$appdir" "$out/codex-gpui-$version-linux-$arch.AppImage" >/dev/null
        ;;
    *) echo "unknown format $format" >&2; exit 2 ;;
    esac
done
ls -1 "$out"
