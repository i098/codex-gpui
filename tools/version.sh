#!/usr/bin/env bash
# Print the app version: `version` in the root Cargo.toml [workspace.package].
# `tools/version.sh MAJOR.MINOR.PATCH` bumps it in Cargo.toml and Cargo.lock and opens the
# CHANGELOG section for it. Tag the merged bump `vMAJOR.MINOR.PATCH` to publish a release.
set -euo pipefail
cd "$(dirname "$0")/.."
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)
if [[ $# -eq 0 ]]; then
    printf '%s\n' "$current"
    exit
fi
next=$1
number='(0|[1-9][0-9]*)'
[[ $# -eq 1 && $next =~ ^$number\.$number\.$number$ ]] || {
    echo 'usage: tools/version.sh [MAJOR.MINOR.PATCH]' >&2
    exit 2
}
[[ $next != "$current" && $(printf '%s\n' "$current" "$next" | sort -V | tail -n 1) == "$next" ]] || {
    echo "$next is not newer than $current" >&2
    exit 2
}
grep -qx '## \[Unreleased\]' CHANGELOG.md || { echo 'CHANGELOG.md has no [Unreleased] section' >&2; exit 2; }
sed "s/^version = \"$current\"$/version = \"$next\"/" Cargo.toml >Cargo.toml.next
mv Cargo.toml.next Cargo.toml
awk -v heading="## [$next] - $(date -u +%F)" '{ print } $0 == "## [Unreleased]" { print ""; print heading }' CHANGELOG.md >CHANGELOG.md.next
mv CHANGELOG.md.next CHANGELOG.md
cargo update --workspace --offline --quiet
printf 'bumped %s to %s; after the bump merges, run: git tag v%s && git push origin v%s\n' "$current" "$next" "$next" "$next"
