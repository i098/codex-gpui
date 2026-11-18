#!/bin/bash
# Ends the processes of run.sh and nothing else: the app tree on its CDP port, the stub and the mock under $R.
R=${R:-/tmp/cgx-refs-offline}
PORT=${PORT:-9495}
for sig in TERM KILL; do
  ps -axo pid=,command= | awk -v port="$PORT" -v dir="$R" -v me="$$" '$1 != me && (index($0, "remote-debugging-port=" port) || index($0, dir "/app/mock/")) { print $1 }' | xargs kill -$sig 2>/dev/null
  sleep 2
done
true
