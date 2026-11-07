#!/bin/bash
# App size on macOS: du, sum of file sizes, and the zip a user downloads (ditto, same method for both apps).
# Usage: tools/bench/size.sh <App.app> [<App.app> ...]
# To build codex-gpui's bundle first (arm64 only: the x86_64 build fails in skia-hairline-sys, which supports Apple silicon only):
#   CODEX_GPUI_DMG=1 bash tools/bundle.sh --assemble /tmp/cgx-bsize/release normal "$(git rev-parse HEAD)" archive
set -eu
for app in "$@"; do
  echo "== $app"
  du -sh "$app"; du -sk "$app"
  echo "filesum_bytes $(find "$app" -type f -print0 | xargs -0 stat -f %z | awk '{s+=$1} END {print s}')"
  zip=$(mktemp -u).zip
  ditto -c -k --keepParent "$app" "$zip"
  echo "zip_bytes $(stat -f %z "$zip")"; rm -f "$zip"
done
