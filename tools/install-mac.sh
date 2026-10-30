#!/usr/bin/env bash
# Run on the Mac (from Linux: tools/mac.sh -l install "bash tools/install-mac.sh $(git rev-parse HEAD)").
# Builds the normal bundle and replaces ~/Applications/Codex GPUI.app in place. It never opens,
# activates or closes the app; a running copy keeps its old files until it restarts.
# shortcut: no update feed, so each update is a manual rerun; use Sparkle (README "In-app updates") when a feed exists.
set -euo pipefail
revision=${1:?usage: tools/install-mac.sh REVISION}
[[ $(uname -s) == Darwin ]] || { echo 'install requires macOS' >&2; exit 2; }
stage_root=/tmp/cgx-mac/install-stage
dest=$HOME/Applications
installed="$dest/Codex GPUI.app"
mkdir -p "$dest"
rm -rf -- "$stage_root"
trap 'rm -rf -- "$stage_root"' EXIT
bash tools/bundle.sh --assemble "$stage_root" normal "$revision"
new=$(echo "$stage_root"/stage.*/"Codex GPUI.app")
[[ -d $new ]] || { echo 'bundle was not assembled' >&2; exit 1; }
rm -rf -- "$dest/.Codex GPUI.new.app" "$dest/.Codex GPUI.previous.app"
ditto "$new" "$dest/.Codex GPUI.new.app"
[[ ! -e $installed ]] || mv "$installed" "$dest/.Codex GPUI.previous.app"
mv "$dest/.Codex GPUI.new.app" "$installed"
codesign --verify --strict "$installed"
printf 'installed %s version %s revision %s\n' "$installed" \
    "$(plutil -extract CFBundleShortVersionString raw "$installed/Contents/Info.plist")" \
    "$(plutil -extract CodexGPUIRevision raw "$installed/Contents/Info.plist")"
