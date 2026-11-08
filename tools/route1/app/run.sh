#!/bin/bash
# Starts the original desktop app in the background on the Mac with a fresh profile: `run.sh <theme>`.
# Everything is local and fake: the mock app-server (fixtures), a signed-in account with placeholder ids
# and an unsigned token, and the loopback https backend stub. The app gets no network apart from
# 127.0.0.1. Scratch lives under $R; stop.sh ends the processes; `rm -rf $R` removes everything.
# The theme is the app's own setting, served by the mock (MOCK_THEME); the Mac's appearance stays untouched.
# Env: R scratch dir (default /tmp/cgx-refs-offline), APP the app bundle, PORT CDP port, STUB_PORT.
set -eu
THEME=${1:?usage: run.sh dark|light}
R=${R:-/tmp/cgx-refs-offline}
APP=${APP:-/tmp/cgx-repin/ChatGPT.app}
PORT=${PORT:-9495}
STUB_PORT=${STUB_PORT:-4750}
D=$R/app
"$D/stop.sh"
rm -rf "$R/prof" "$D/mock.log" "$D/stub.log"
mkdir -p "$R/prof/home" "$R/prof/codex-home" "$R/prof/userdata"
[ -f "$R/cert.pem" ] || openssl req -x509 -newkey rsa:2048 -nodes -keyout "$R/key.pem" -out "$R/cert.pem" -days 30 -subj /CN=127.0.0.1 2>/dev/null
# The fixture threads ran in /tmp/cgx-mac/proj: a git repository whose working tree holds the subtract change.
P=/tmp/cgx-mac/proj
rm -rf "$P"
mkdir -p "$P/src"
printf 'def add(a, b):\n    return a + b\n' >"$P/src/math.py"
printf 'print("Hello, world")\n' >"$P/hello.py"
git -C "$P" init -q
git -C "$P" add -A
git -C "$P" -c user.name=Demo -c user.email=user@example.com commit -qm "Initial commit"
printf 'def add(a, b):\n    return a + b\n\n\ndef subtract(a, b):\n    return a - b\n' >"$P/src/math.py"
/opt/homebrew/bin/node "$D/seed.mjs" "$D/mock/oracle.json" "$R/prof/codex-home"
# Stub and app start through launchctl asuser (the console user's session gives the app a window server and
# keeps both alive after the ssh session ends); the app stays in the background.
sudo -n launchctl asuser "$(id -u)" sudo -n -u "$USER" env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin \
  R1_ORACLE="$D/mock/oracle.json" TLS_KEY="$R/key.pem" TLS_CERT="$R/cert.pem" STUB_LOG="$D/stub.log" \
  /opt/homebrew/bin/node "$D/mock/backend-stub.mjs" "$STUB_PORT" >"$D/stub.out" 2>&1 </dev/null &
sleep 1
sudo -n launchctl asuser "$(id -u)" sudo -n -u "$USER" env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin USER="$USER" LOGNAME="$USER" TZ=America/Los_Angeles TMPDIR=/tmp \
  HOME="$R/prof/home" CODEX_HOME="$R/prof/codex-home" CODEX_ELECTRON_USER_DATA_PATH="$R/prof/userdata" CODEX_ELECTRON_START_IN_BACKGROUND=1 \
  CODEX_CLI_PATH="$D/codex" APP="$APP" STUB_PORT="$STUB_PORT" R1_THEME="$THEME" \
  "$APP/Contents/MacOS/ChatGPT" --remote-debugging-port="$PORT" --remote-debugging-address=127.0.0.1 \
  --user-data-dir="$R/prof/userdata" --force-device-scale-factor=2 --num-raster-threads=1 --disable-checker-imaging --disable-partial-raster \
  --disable-gpu --disable-gpu-compositing --disable-gpu-rasterization \
  --disable-backgrounding-occluded-windows --disable-renderer-backgrounding --disable-background-timer-throttling \
  --ignore-certificate-errors '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1' \
  >"$D/app.log" 2>&1 </dev/null &
echo "app starting on CDP $PORT"
