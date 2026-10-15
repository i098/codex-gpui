#!/usr/bin/env bash
# Text-rasterization lab: renders the runs of crates/textsys/examples/measure.rs with
# chrome-headless-shell (as tools/capture does for refs) and with GPUI on the Mac, then prints
# the per-run inside-text-box diff table (before = stock AppleTextSystem, after = textsys).
# Usage: tools/text/measure.sh   Results: /tmp/cgx/ptext/text/{table.md,chrome,before,after,diff}.png
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
lane=ptext remote=/tmp/cgx-mac dir=target/text
cargo="cargo run -q -p textsys --example measure --"
chs="$remote/chrome-headless-shell-mac-arm64/chrome-headless-shell --headless --user-data-dir=$remote/chs-text"
# The Mac drops SSH handshakes under load (exit 255): retry those.
mac() { for _ in 1 2 3 4 5 6 7 8 9 10; do tools/mac.sh -l $lane "$1" && return; [[ $? == 255 ]] || return 1; sleep 15; done; return 1; }
mac "set -e; mkdir -p $dir; cd $dir; d=\$PWD; cd - >/dev/null
  $cargo page $dir > $dir/size; read w h < $dir/size
  $chs --hide-scrollbars --force-device-scale-factor=2 --window-size=\$w,\$h --virtual-time-budget=3000 --screenshot=\$d/chrome.png file://\$d/page.html 2>/dev/null
  $chs --force-device-scale-factor=2 --window-size=\$w,\$h --virtual-time-budget=3000 --dump-dom file://\$d/page.html > \$d/dom.html 2>/dev/null
  rm -rf $remote/chs-text
  $cargo compare $dir"
mkdir -p /tmp/cgx/ptext/text
until rsync -a "mac:$remote/src/$lane/$dir/" /tmp/cgx/ptext/text/; do sleep 15; done
