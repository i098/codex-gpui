#!/bin/bash
# Fake `codex` for both apps: startup-config probes go to the original's bundled CLI, `app-server` is the recorded-fixture mock.
# Run dir and repo come from files that perf.mjs writes, because the QA build clears the environment.
W=$(cd "$(dirname "$0")" && pwd)
OUT=$(cat "$W/current-out"); REPO=$(cat "$W/current-repo"); ORIG=$(cat "$W/current-orig")
case " $* " in
  *" app-server "*) exec env MOCK_FIXTURES="$REPO/fixtures" R1_ORACLE="$REPO/tools/route1/oracle.json" MOCK_LOG="$OUT/mock.log" MOCK_LOG_EPOCH=1 node "$REPO/tools/route1/linux/mock-app-server.mjs" ;;
  *) exec "$ORIG/Contents/Resources/codex-cli/bin/codex" "$@" ;;
esac
