#!/usr/bin/env bash
# Build codex-gpui-<version>-x86_64.AppImage from a release binary. Needs appimagetool on PATH.
# Usage: tools/appimage.sh <binary> <out.AppImage>
set -euo pipefail
cd "$(dirname "$0")/.."
bin=$(realpath "$1")
out=$(realpath -m "$2")
dir=$(mktemp -d)
trap 'rm -rf -- "$dir"' EXIT
install -D "$bin" "$dir/usr/bin/codex-gpui"
cat >"$dir/codex-gpui.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Codex
Exec=codex-gpui
Icon=codex-gpui
Categories=Development;
DESKTOP
printf '#!/bin/sh\nexec "$APPDIR/usr/bin/codex-gpui" "$@"\n' >"$dir/AppRun"
chmod +x "$dir/AppRun"
# The icon is the first PNG chunk inside AppIcon.icns.
python3 - assets/app/AppIcon.icns "$dir/codex-gpui.png" <<'PY'
import sys
data = open(sys.argv[1], 'rb').read()
start = data.index(b'\x89PNG\r\n\x1a\n')
end = data.index(b'IEND', start) + 8
open(sys.argv[2], 'wb').write(data[start:end])
PY
ARCH=x86_64 appimagetool "$dir" "$out"
