#!/usr/bin/env bash
# Run a command in a copy of this checkout on the macOS arm64 build host (`ssh mac`).
# All state lives under /tmp/cgx-mac on the Mac (one target dir per lane, so lanes never
# run each other's binaries). Usage: tools/mac.sh [-l lane] <command...>
set -euo pipefail
lane=main
if [[ ${1:-} == -l ]]; then lane=$2; shift 2; fi
root=$(git rev-parse --show-toplevel)
remote=/tmp/cgx-mac
rsync -a --delete --exclude target --exclude .git "$root/" "mac:$remote/src/$lane/"
ssh mac "export RUSTUP_HOME=$remote/rustup CARGO_HOME=$remote/cargo CARGO_TARGET_DIR=$remote/target/$lane CARGO_NET_GIT_FETCH_WITH_CLI=true PATH=$remote/cargo/bin:\$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin; [ -d \$CARGO_TARGET_DIR ] || { mkdir -p \$CARGO_TARGET_DIR && cp -Rc $remote/target/debug \$CARGO_TARGET_DIR/ 2>/dev/null; }; cd $remote/src/$lane && $*"
