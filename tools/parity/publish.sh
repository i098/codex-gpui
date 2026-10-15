#!/usr/bin/env bash
# Upload a parity report dir (report.html/json + ref/port/diff PNGs) to the public R2
# evidence bucket and print a Markdown block for a PR body.
# Usage: tools/parity/publish.sh <out_dir>
# Needs AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, R2_EP, AWS_DEFAULT_REGION=auto exported.
set -euo pipefail
out=${1:?usage: publish.sh <out_dir>}
[[ -f $out/report.json && -f $out/report.html ]] || { echo "no report in $out" >&2; exit 1; }
: "${AWS_ACCESS_KEY_ID:?}" "${AWS_SECRET_ACCESS_KEY:?}" "${R2_EP:?}"
key=$(openssl rand -hex 24)
url=https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/$key
uvx --from awscli aws s3 cp --recursive --only-show-errors --endpoint-url "$R2_EP" \
  --exclude '*' --include 'report.html' --include 'report.json' --include '*.png' \
  "$out" "s3://swarms-pr-evidence/$key/" >&2

echo "**Parity report:** $url/report.html"
echo
echo '| screen | theme | size | outside px | text ratio | result | ref | port | diff |'
echo '|---|---|---|---|---|---|---|---|---|'
jq -r --arg u "$url" '.[] | "| \(.screen) | \(.theme) | \(.width)x\(.height) | \(.diff_px_outside_text) | \(.text_diff_ratio * 100000 | round / 1000)% | \(if .pass then "PASS" else "FAIL" end) | ![](\($u)/\(.ref_png)) | ![](\($u)/\(.port_png)) | ![](\($u)/\(.diff_png)) |"' "$out/report.json"
