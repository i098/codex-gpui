#!/bin/sh
# Linux state discovery only; final references use the pinned Mac renderer.
# Set R1_ORACLE to an owned copy before starting this server.
set -eu
: "${R1_ORACLE:?Set R1_ORACLE to an owned writable oracle copy}"
export R1_ORACLE
export MOCK_LOG="${MOCK_LOG:-$(dirname "$R1_ORACLE")/app-server.log}"
export R1_OFFLINE=1 R1_SF_FONTS=1 R1_TRACE=1
export R1_PORT="${1:-4561}"
exec node "$(dirname "$0")/../server.mjs"
