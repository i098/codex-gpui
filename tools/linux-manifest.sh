#!/usr/bin/env bash
# Write the signed update manifest the Linux and Windows updaters read (`latest.json`).
# Usage: tools/linux-manifest.sh <package> <version> <download-url> <out.json>
# Needs CODEX_GPUI_UPDATE_PRIVATE_KEY: an Ed25519 private key in PEM form
# (`openssl genpkey -algorithm ed25519`). The app trusts the matching public key, given to the
# build as base64 of the raw 32 bytes: `openssl pkey -in key.pem -pubout -outform DER | tail -c 32 | base64`.
# The signature covers the raw SHA-256 digest of the package.
set -euo pipefail
package=$1 version=$2 url=$3 out=$4
: "${CODEX_GPUI_UPDATE_PRIVATE_KEY:?set CODEX_GPUI_UPDATE_PRIVATE_KEY}"
key=$(mktemp) digest=$(mktemp)
trap 'rm -f -- "$key" "$digest"' EXIT
printf '%s\n' "$CODEX_GPUI_UPDATE_PRIVATE_KEY" >"$key"
sha=$(openssl dgst -sha256 -binary "$package" | xxd -p -c 64)
printf '%s' "$sha" | xxd -r -p >"$digest"
signature=$(openssl pkeyutl -sign -rawin -in "$digest" -inkey "$key" | base64 -w0)
printf '{"version":"%s","url":"%s","sha256":"%s","signature":"%s"}\n' "$version" "$url" "$sha" "$signature" >"$out"
