#!/usr/bin/env bash
# Rebuilds the Skia binaries crates/shapes links (skia-safe, macOS arm64) with Chromium's
# SkUserConfig legacy defines (chromium/src skia/config/SkUserConfig.h), so Skia's CPU raster
# matches chrome-headless-shell bit for bit, and uploads the archive for SKIA_BINARIES_URL
# (.cargo/config.toml). Stock skia-safe binaries lack SK_SUPPORT_LEGACY_CONIC_CHOP and differ.
# Usage: tools/shapes/skia.sh   (needs /tmp/cgx/r2env.sh; bump the skia-safe version in
# crates/shapes/Cargo.toml and here together)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
version=0.153.3 remote=/tmp/cgx-mac/skia-build
defines='"-DSK_SUPPORT_LEGACY_CONIC_CHOP","-DSK_USE_PADDED_BLUR_UPSCALE","-DSK_LEGACY_INITWITHPREV_LAYER_SIZING","-DSK_AVOID_SLOW_RASTER_PIPELINE_BLURS","-DSK_SUPPORT_LEGACY_RRECT_TRANSFORM","-DSK_SUPPORT_LEGACY_ANISOTROPIC_MIPMAP_SCALE","-DSK_USE_SAFE_INSET_FOR_TEXTURE_SAMPLING"'
ssh mac "set -e; mkdir -p $remote/src /tmp/cgx-mac/bin; cd $remote
  [ -x /tmp/cgx-mac/bin/ninja ] || (cd /tmp/cgx-mac/bin && curl -sfLO https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-mac.zip && unzip -oq ninja-mac.zip)
  printf '[package]\nname = \"skiabuild\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[dependencies]\nskia-safe = { version = \"=$version\", default-features = false, features = [\"binary-cache\"] }\n[workspace]\n' > Cargo.toml
  echo 'fn main() {}' > src/main.rs
  export RUSTUP_HOME=/tmp/cgx-mac/rustup CARGO_HOME=/tmp/cgx-mac/cargo PATH=/tmp/cgx-mac/bin:/tmp/cgx-mac/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin
  FORCE_SKIA_BUILD=1 SKIA_GN_ARGS='extra_cflags_cc=[$defines]' cargo build -q --release
  out=\$(dirname \$(ls target/release/build/skia-bindings-*/out/skia/libskia.a))
  rm -rf stage && mkdir -p stage/skia-binaries && cp \$out/*.a \$out/bindings.rs \$out/skia-defines.txt stage/skia-binaries/
  tar -C stage -czf skia-binaries.tar.gz skia-binaries"
# The archive name carries skia-bindings' cache key: read it from a build that misses.
key=$(ssh mac "cd $remote && export RUSTUP_HOME=/tmp/cgx-mac/rustup CARGO_HOME=/tmp/cgx-mac/cargo PATH=/tmp/cgx-mac/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin; CARGO_TARGET_DIR=target-key SKIA_BINARIES_URL='file:///nonexistent/{key}' cargo build -vv 2>&1 | grep -o 'file:///nonexistent/[^ ]*' | head -1 | sed 's|.*/||'" || true)
[[ -n $key ]] || { echo "could not read the skia-binaries key" >&2; exit 1; }
dir=skia-chromium-$(openssl rand -hex 24)
scp -q "mac:$remote/skia-binaries.tar.gz" "/tmp/skia-binaries-$key.tar.gz"
(. /tmp/cgx/r2env.sh; uvx -q --from awscli aws s3 cp "/tmp/skia-binaries-$key.tar.gz" \
  "s3://swarms-pr-evidence/$dir/$version/skia-binaries-$key.tar.gz" --endpoint-url "$R2_EP" >/dev/null)
echo "SKIA_BINARIES_URL = \"https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/$dir/{tag}/skia-binaries-{key}.tar.gz\""
