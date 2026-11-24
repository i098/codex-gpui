#!/usr/bin/env bash
# Print the tag to publish for the workspace version, or nothing. Only a 0.1.x patch publishes
# automatically; any other version needs a manual `git tag` and push. `tools/auto-release-tag.sh test` self-checks the gate.
set -euo pipefail
gate() { [[ $1 =~ ^0\.1\.(0|[1-9][0-9]*)$ ]] && echo "v$1" || true; }
if [[ ${1:-} == test ]]; then
    [[ $(gate 0.1.5) == v0.1.5 && $(gate 0.1.10) == v0.1.10 ]] || { echo 'gate rejects a 0.1.x patch' >&2; exit 1; }
    for v in 0.2.0 1.1.5 0.1 0.1.05 0.1.5-rc1 0.10.1 0.1.5.1; do
        [[ -z $(gate "$v") ]] || { echo "gate accepts $v" >&2; exit 1; }
    done
    echo ok
    exit
fi
gate "$("$(dirname "$0")/version.sh")"
