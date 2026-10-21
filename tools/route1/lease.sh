#!/usr/bin/env bash
set -euo pipefail
if (($# == 0)); then
  echo "Usage: tools/route1/lease.sh <capture command> [args...]" >&2
  exit 64
fi
lock=${R1_CAPTURE_LOCK:-/sdc-disk/tmp/cgx/cgx-capture-service/capture.lock}
mkdir -p "$(dirname "$lock")"
exec flock --conflict-exit-code 75 --wait "${R1_CAPTURE_WAIT:-300}" "$lock" \
  timeout --kill-after=5 "${R1_CAPTURE_TIMEOUT:-180}" "$@"
