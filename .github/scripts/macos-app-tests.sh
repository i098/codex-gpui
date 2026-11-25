#!/usr/bin/env bash
# Runs `cargo test -p app --test <t>` for each target name on stdin, one at a time, and deletes each
# test binary afterwards (~150 linked test binaries fill the runner disk). Prints the totals; fails if any target fails.
set -uo pipefail
echo "runner: $(hostname) $RUNNER_NAME"; df -h . /tmp
log=$(mktemp /tmp/macos-app-tests.XXXXXX)
failed=""
while read -r t; do
  [ -n "$t" ] || continue
  # perl alarm: one hung target must not eat the whole job. Skipped test: skipped by exact name, tracked in #1650.
  perl -e 'alarm 900; exec @ARGV' cargo test --locked -p app --test "$t" -- --test-threads=1 \
    --skip sound_modes_round_trip_in_an_owned_native_preferences_domain 2>&1 | tee -a "$log" || failed="$failed $t"
  rm -f target/debug/deps/"$t"-*
done
awk '/^test result/ {p+=$4; f+=$6} END {print "macOS app tests passed=" p " failed=" f}' "$log"
rm -f "$log"
if [ -n "$failed" ]; then echo "failed targets:$failed"; exit 1; fi
