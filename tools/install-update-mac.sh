#!/bin/bash
set -euo pipefail

archive_url=${1:?usage: install-update-mac.sh ARCHIVE_URL APP_BUNDLE}
installed=${2:?usage: install-update-mac.sh ARCHIVE_URL APP_BUNDLE}
[[ $archive_url == https://*.zip && -d $installed && $installed == *.app ]] || exit 2

previous="$(dirname "$installed")/.Codex GPUI.previous.app"
stage=$(mktemp -d "${TMPDIR:-/tmp}/codex-gpui-update.XXXXXX")
cleanup() {
    status=$?
    if (( status != 0 )) && [[ ! -e $installed && -e $previous ]]; then
        mv "$previous" "$installed" || true
    fi
    rm -rf -- "$stage" || true
    exit "$status"
}
trap cleanup EXIT
archive=$stage/update.zip
curl --fail --location --silent --show-error --proto '=https' --proto-redir '=https' "$archive_url" -o "$archive"
curl --fail --location --silent --show-error --proto '=https' --proto-redir '=https' "${archive_url}.sha256" -o "$archive.sha256"
read -r expected _ <"$archive.sha256"
actual=$(shasum -a 256 "$archive" | cut -d ' ' -f 1)
[[ $expected == "$actual" ]] || { echo 'update checksum mismatch' >&2; exit 1; }
ditto -x -k "$archive" "$stage"
new="$stage/Codex GPUI.app"
[[ -d $new ]] || { echo 'update archive has no Codex GPUI.app' >&2; exit 1; }
codesign --verify --deep --strict "$new"
rm -rf -- "$previous"
mv "$installed" "$previous"
mv "$new" "$installed"
open -g -n "$installed"
