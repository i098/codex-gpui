#!/usr/bin/env bash
# crates/shapes lab: refreshes tools/shapes/cases.json from the refs, renders the verification
# page in Chromium and GPUI (before = GPUI quads/shadows, after = crates/shapes) and prints the
# per-decoration diff table.
# Results: /tmp/cgx/pshapes/{table.md,verify,before,after,before-diff,after-diff}.png
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
lane=pskia remote=/tmp/cgx-mac dir=target/shapes out=/tmp/cgx/pshapes
lab="cargo run -q -p shapes --features lab --example lab --"
chs="$remote/chrome-headless-shell-mac-arm64/chrome-headless-shell --headless --user-data-dir=$remote/chs-$lane --hide-scrollbars --force-device-scale-factor=2 --virtual-time-budget=2000"
# The Mac drops SSH handshakes under load (exit 255): retry those.
mac() { for _ in 1 2 3 4 5 6 7 8 9 10; do tools/mac.sh -l $lane "$1" && return; [[ $? == 255 ]] || return 1; sleep 15; done; return 1; }
python3 tools/shapes/inventory.py >/dev/null
mac "set -e; mkdir -p $dir; d=\$PWD/$dir; read vw vh < <($lab pages $dir)
  $chs --window-size=\$vw,\$vh --screenshot=\$d/verify.png file://\$d/verify.html 2>/dev/null
  rm -rf $remote/chs-$lane
  $lab verify $dir > $dir/table.md 2> $dir/details.log"
mkdir -p $out
until rsync -a "mac:$remote/src/$lane/$dir/" $out/; do sleep 15; done
cat $out/table.md
