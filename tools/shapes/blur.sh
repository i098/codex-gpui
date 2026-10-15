#!/usr/bin/env bash
# crates/shapes backdrop-blur lab: renders the refs' `backdrop-filter: blur()` scenes in Chromium
# and GPUI (before = plain shapes::decoration, after = shapes::backdrop_blur) and prints the
# per-box diff table. Results: /tmp/cgx/pblur-lab/{table.md,blur-<theme>*.png}
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
lane=${LANE:-pblur} remote=/tmp/cgx-mac dir=target/blur out=/tmp/cgx/pblur-lab
lab="cargo run -q -p shapes --features lab --example blur --"
chs="$remote/chrome-headless-shell-mac-arm64/chrome-headless-shell --headless --user-data-dir=$remote/chs-$lane --hide-scrollbars --force-device-scale-factor=2 --virtual-time-budget=2000 --window-size=1280,832"
mac() { for _ in 1 2 3 4 5 6 7 8 9 10; do tools/mac.sh -l "$lane" "$1" && return; [[ $? == 255 ]] || return 1; sleep 15; done; return 1; }
mac "set -e; mkdir -p $dir; d=\$PWD/$dir; $lab pages $dir
  for t in dark light; do $chs --screenshot=\$d/blur-\$t.png file://\$d/blur-\$t.html 2>/dev/null; done
  rm -rf $remote/chs-$lane
  $lab verify $dir > $dir/table.md"
mkdir -p $out
until rsync -a "mac:$remote/src/$lane/$dir/" $out/; do sleep 15; done
cat $out/table.md
