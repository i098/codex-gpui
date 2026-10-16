#!/bin/sh
# route1 on Linux, no Mac: the original Codex renderer in headless Chrome, driven by lib.mjs.
#
#   sh tools/route1/linux/start.sh [port]        # 1. server + mock app-server, default port 4561
#   fleet-browser up chrome                      # 2. headless Chrome with CDP on :9522
#   node tools/route1/linux/lib.mjs record <out.json>   # 3. record 01 + 09 twice (determinism run)
#
# What it does: copies tools/route1 to /tmp/inv-r1 (the server never rewrites the tracked
# oracle.json), links fixtures and tools/capture, writes a fake `ssh` that runs
# mock-app-server.mjs (JSON-RPC from fixtures/readback.jsonl + fixtures/sessions/*.jsonl; unknown
# methods answer -32601; log in /tmp/inv-r1/mock-app-server.log), then starts
# `R1_OFFLINE=1 R1_SF_FONTS=1 R1_TRACE=1 node tools/route1/server.mjs` with CODEX_HOME=/tmp/inv-r1/home.
#
# lib.mjs sets each page up with: Mac UA, America/Los_Angeles, 1280x832@1, dark theme, hover media
# emulation (headless Chrome reports hover:none), a request log (window.__r1req), a native
# context-menu hook (window.__ctx), and auto-dismissal of the model announcement modal.
#
# Limits: native menu accelerators never fire from real keys (send {type:'run-command', id});
# navigator.platform stays Linux, so ProseMirror Mod is Ctrl; writes (rename, archive, pin, send)
# are refused; thread/resume answers "no recorded response".
set -e
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
PORT=${1:-4561}
R=/tmp/inv-r1
mkdir -p $R/bin $R/home $R/route1/tools
rm -rf $R/route1/tools/route1 && cp -r "$REPO/tools/route1" $R/route1/tools/route1
ln -sfn "$REPO/fixtures" $R/route1/fixtures
ln -sfn "$REPO/tools/capture" $R/route1/tools/capture
cat > $R/bin/ssh <<EOS
#!/bin/sh
# fake ssh: ignores host and command, runs the local mock app-server
exec node "$REPO/tools/route1/linux/mock-app-server.mjs"
EOS
chmod +x $R/bin/ssh
: > $R/mock-app-server.log
cd $R/route1
PATH=$R/bin:$PATH CODEX_HOME=$R/home R1_OFFLINE=1 R1_SF_FONTS=1 R1_TRACE=1 R1_PORT=$PORT exec node tools/route1/server.mjs
