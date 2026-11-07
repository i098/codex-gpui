#!/bin/bash
# Interleaved benchmark of the original Codex app against codex-gpui on macOS. 5 rounds; the app order alternates each round.
# Usage: tools/bench/run.sh plain|thread
#   plain  no thread link: cold then warm launch per app per round (series A)
#   thread open codex://threads/<id>: cold launch per app per round (series C)
# Env (required): BENCH_ORIG = the original ChatGPT.app; BENCH_BIN = codex-gpui built with CODEX_GPUI_QA=1 and probe.diff applied.
# Env (optional): BENCH_WORK = scratch dir (default /tmp/cgx-bench); BENCH_PORT = CDP port for the original (default 9493).
# Every run is "visible-emulated" (--vis), so a locked console still paints. Results: $BENCH_WORK/runs/<series>-<app>-<kind>-<n>/result.json.
set -eu
state=${1:?usage: run.sh plain|thread}
: "${BENCH_ORIG:?}" "${BENCH_BIN:?}"
HERE=$(cd "$(dirname "$0")" && pwd)
export BENCH_WORK=${BENCH_WORK:-/tmp/cgx-bench} BENCH_ORIG
PORT=${BENCH_PORT:-9493}
export PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
OUT=$BENCH_WORK/runs
mkdir -p "$OUT"
install -m 755 "$HERE/codex-wrapper.sh" "$BENCH_WORK/codex"
ORIG_EXE=$(realpath "$BENCH_ORIG")/Contents/MacOS/ChatGPT
BIN=$(realpath "$BENCH_BIN")

stop() { # stop only the apps this script started
  sel() { ps -axo pid=,command= | awk -v o="$ORIG_EXE" -v b="$BIN" -v p="--remote-debugging-port=$PORT" '($2 == o && index($0, p)) || $2 == b {print $1}'; }
  for p in $(sel); do kill "$p"; done
  for _ in $(seq 100); do [ -z "$(sel)" ] && return 0; sleep 0.2; done
  echo "still running: $(sel)" >&2
}
run() { # app dir flags...
  local app=$1 dir=$2; shift 2
  if [ "$app" = ours ]; then node "$HERE/perf.mjs" ours "$OUT/$dir" --bin "$BIN" "$@"; else node "$HERE/perf.mjs" orig "$OUT/$dir" --port "$PORT" "$@"; fi
  stop; uptime >> "$OUT/load.log"; sleep 3
}

if [ "$state" = plain ]; then run orig prime-orig --vis; run ours prime-ours --vis; fi # not counted: first launch creates the profile
for r in 1 2 3 4 5; do
  if [ $((r % 2)) = 1 ]; then order="orig ours"; else order="ours orig"; fi
  for app in $order; do
    if [ "$state" = plain ]; then
      run "$app" "A-$app-cold-$r" --cold --vis
      run "$app" "A-$app-warm-$r" --vis
    else
      run "$app" "C-$app-thread-$r" --cold --vis --thread
    fi
  done
done
python3 "$HERE/analyze.py" "$OUT"
