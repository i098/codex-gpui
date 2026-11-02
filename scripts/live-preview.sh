#!/usr/bin/env bash
# Live preview: watch the source, rebuild, re-render core screens headless to PNG, serve a page
# that polls and swaps images. Usage: scripts/live-preview.sh [port]   (default 18500)
# Env: LIVE_OUT (output dir), CARGO_TARGET_DIR, LIVE_POLL (seconds between source scans).
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd -P)
PORT=${1:-${LIVE_PORT:-18500}}
OUT=${LIVE_OUT:-/sdc-disk/tmp/cgx/live-preview}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/sdc-disk/tmp/cgx/cargo-targets/$(echo "$REPO" | tr / _ | sed 's/^_//')}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-16}
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"  # systemd --user starts with a minimal PATH
POLL=${LIVE_POLL:-2}
cd "$REPO" || exit 1
mkdir -p "$OUT/img"
cp "$REPO/scripts/live-preview.html" "$OUT/index.html"

# name:example:args  (the example writes to its cwd as shot.png or the path given)
SCREENS=("home:shot:" "usage:usage:img/usage.png" "settings:settings_kit:dark closed img/settings.png")

status() { # state error-text-file
  local n=0 shots=""
  for s in "${SCREENS[@]}"; do
    f="$OUT/img/${s%%:*}.png"
    [ -f "$f" ] && shots="$shots${shots:+,}{\"name\":\"${s%%:*}\",\"v\":\"$(stat -c %Y.%s "$f")\"}"
  done
  python3 - "$1" "$OUT" "$shots" "${BUILD_STARTED:-0}" "${LAST_OK:-0}" "${BUILD_SECS:-0}" <<'PY'
import json, sys, os
state, out, shots, started, ok, secs = sys.argv[1:7]
log = ""
if state == "failed":
    log = open(out + "/build.log", errors="replace").read()[-12000:]
d = {"state": state, "started": int(started), "last_ok": int(ok), "secs": int(secs),
     "shots": json.loads("[" + shots + "]"), "error": log}
json.dump(d, open(out + "/status.tmp", "w"))
os.replace(out + "/status.tmp", out + "/status.json")
PY
}

build() {
  BUILD_STARTED=$(date +%s); status building
  local ex=()
  for s in "${SCREENS[@]}"; do IFS=: read -r _ e _ <<<"$s"; ex+=(--example "$e"); done
  if ! flock /tmp/cargo-slot.lock nice -n10 ionice -c3 cargo build -p ui "${ex[@]}" --color never >"$OUT/build.log" 2>&1; then
    BUILD_SECS=$(( $(date +%s) - BUILD_STARTED )); status failed; return
  fi
  local d="$CARGO_TARGET_DIR/debug/examples" r="$OUT/render"
  mkdir -p "$r/img"
  for s in "${SCREENS[@]}"; do
    IFS=: read -r name e a <<<"$s"
    ( cd "$r" && "$d/$e" $a >>"$OUT/build.log" 2>&1 ) || { BUILD_SECS=$(( $(date +%s) - BUILD_STARTED )); status failed; return; }
    [ "$name" = home ] && mv "$r/shot.png" "$r/img/home.png"
  done
  # Publish images only when every screen rendered; cmp keeps the version stable for unchanged PNGs.
  for f in "$r"/img/*.png; do cmp -s "$f" "$OUT/img/$(basename "$f")" || cp "$f" "$OUT/img/"; done
  LAST_OK=$(date +%s); BUILD_SECS=$((LAST_OK - BUILD_STARTED)); status ok
}

sig() {
  find crates assets fixtures Cargo.toml Cargo.lock -type f -not -path '*/target/*' \
    -printf '%T@ %p\n' 2>/dev/null | sha1sum
}

python3 -m http.server "$PORT" --bind 127.0.0.1 -d "$OUT" >/dev/null 2>&1 &
trap 'kill $!' EXIT
LAST_OK=0; BUILD_SECS=0
[ -f "$OUT/status.json" ] && LAST_OK=$(python3 -c "import json;print(json.load(open('$OUT/status.json'))['last_ok'])")
echo "serving http://127.0.0.1:$PORT from $OUT"
prev=""
while :; do
  cur=$(sig)
  if [ "$cur" != "$prev" ]; then
    prev=$cur
    build
    prev=$(sig) # edits made during the build trigger the next round only if they change the tree again
    [ "$prev" != "$cur" ] && prev=""
  fi
  sleep "$POLL"
done
