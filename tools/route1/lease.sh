#!/usr/bin/env bash
set -euo pipefail
if (($# == 0)); then
  echo "Usage: tools/route1/lease.sh <capture command> [args...]" >&2
  exit 64
fi
lock=${R1_CAPTURE_LOCK:-/sdc-disk/tmp/cgx/cgx-capture-service/capture.lock}
mkdir -p "$(dirname "$lock")"
IFS=, read -r -a endpoints <<< "${R1_CAPTURE_CDP_URLS:-http://127.0.0.1:29484,http://127.0.0.1:29485,http://127.0.0.1:29486}"
exec {scheduler}>"$lock.scheduler"
flock "$scheduler"
slot=0
if [[ -s "$lock.next" ]]; then read -r slot < "$lock.next" || slot=0; fi
[[ $slot =~ ^[0-9]+$ ]] || slot=0
slot=$((slot % ${#endpoints[@]}))
printf '%s\n' "$(((slot + 1) % ${#endpoints[@]}))" > "$lock.next"
flock -u "$scheduler"
exec {scheduler}>&-
exec flock --conflict-exit-code 75 --wait "${R1_CAPTURE_WAIT:-1800}" "$lock.$slot" \
  env R1_CDP="${endpoints[$slot]}" timeout --kill-after=5 "${R1_CAPTURE_TIMEOUT:-180}" "$@"
