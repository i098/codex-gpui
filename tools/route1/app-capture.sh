#!/bin/bash
# Captures reference screens from the original desktop app, run on a Mac, without any login:
#   R1_APP_HOST=user@mac [R1_APP_SSH_OPTS="-i key"] tools/route1/app-capture.sh <out-dir> [screen...]
# For each theme it copies the mock, the backend stub and the scrubbed oracle to the Mac, starts the app
# in the background (app/run.sh), reaches the CDP port through an ssh tunnel and runs capture-all.mjs
# against it (R1_APP_CDP). The app is stopped afterwards and the scratch directory is deleted.
set -eu
OUT=${1:?usage: app-capture.sh <out-dir> [screen...]}
shift
HOST=${R1_APP_HOST:?set R1_APP_HOST to the ssh destination of the Mac}
OPTS=${R1_APP_SSH_OPTS:-}
R=${R:-/tmp/cgx-refs-offline}
PORT=${PORT:-9495}
LOCAL=${R1_APP_LOCAL_PORT:-29495}
here=$(cd "$(dirname "$0")" && pwd)
root=$here/../..
ssh_mac() { ssh $OPTS -o BatchMode=yes "$HOST" "$@"; }

stage=$(mktemp -d)
tunnel=
cleanup() {
  [ -z "$tunnel" ] || kill "$tunnel" 2>/dev/null || true
  ssh_mac "R=$R PORT=$PORT $R/app/stop.sh; rm -rf $R /tmp/cgx-mac/proj; rmdir /tmp/cgx-mac" 2>/dev/null || true
  rm -rf "$stage"
}
trap cleanup EXIT

mkdir -p "$stage/app/mock/linux"
cp "$here"/app/* "$stage/app/"
cp -r "$root/fixtures" "$stage/app/mock/fixtures"
cp "$here/linux/mock-app-server.mjs" "$stage/app/mock/linux/"
cp "$here/backend-stub.mjs" "$here/oracle.json" "$stage/app/mock/"
ssh_mac "rm -rf $R && mkdir -p $R"
tar -C "$stage" -cf - app | ssh_mac "tar -C $R -xf -"

# One launch per run: a follow-up turn (terminal) stays in the app's thread, so each terminal capture needs its own.
launch() {
  ssh_mac "R=$R PORT=$PORT $R/app/run.sh $1"
  ssh $OPTS -o BatchMode=yes -o ExitOnForwardFailure=yes -N -L "$LOCAL:127.0.0.1:$PORT" "$HOST" &
  tunnel=$!
  for _ in $(seq 1 60); do
    curl -fs "http://127.0.0.1:$LOCAL/json" 2>/dev/null | grep -q '"app://' && break
    sleep 2
  done
}
land() {
  kill "$tunnel"
  tunnel=
  ssh_mac "R=$R PORT=$PORT $R/app/stop.sh"
}
screens=("$@")
[ ${#screens[@]} -gt 0 ] || screens=(home sidebar thread composer diff settings automations terminal mobile)
rest=()
terminal=
for screen in "${screens[@]}"; do
  if [ "$screen" = terminal ]; then terminal=1; else rest+=("$screen"); fi
done
rc=0
for theme in ${R1_THEMES:-dark light}; do
  if [ ${#rest[@]} -gt 0 ]; then
    launch "$theme"
    R1_APP_CDP="http://127.0.0.1:$LOCAL" R1_THEMES=$theme R1_OUT="$OUT" timeout 900 node "$here/capture-all.mjs" "${rest[@]}" || rc=1
    land
  fi
  if [ -n "$terminal" ]; then
    for size in "1280 832" "1440 900"; do
      launch "$theme"
      R1_APP_CDP="http://127.0.0.1:$LOCAL" R1_OUT="$OUT" timeout 300 node "$here/capture.mjs" terminal "$theme" $size || rc=1
      land
    done
  fi
done
exit $rc
