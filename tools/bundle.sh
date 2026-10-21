#!/usr/bin/env bash
# Build and verify a bundle without launching it.
set -euo pipefail
if [[ ${1:-} != --assemble ]]; then
    cd "$(git rev-parse --show-toplevel)"
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/app/Cargo.toml)
    mode=${1:-normal}
    [[ $mode == normal || $mode == --qa ]] || { echo 'usage: tools/bundle.sh [--qa]' >&2; exit 2; }
    revision=$(git rev-parse HEAD)
    [[ -z $(git status --porcelain) ]] || revision=$revision-dirty
    lane=bundle
    [[ $mode == normal ]] || { mode=qa; lane=qa-sync; }
    printf -v update_env 'CODEX_GPUI_UPDATE_FEED=%q CODEX_GPUI_UPDATE_PUBLIC_KEY=%q CODEX_GPUI_SPARKLE_FRAMEWORK=%q CODEX_GPUI_SIGN_IDENTITY=%q' \
        "${CODEX_GPUI_UPDATE_FEED:-}" "${CODEX_GPUI_UPDATE_PUBLIC_KEY:-}" \
        "${CODEX_GPUI_SPARKLE_FRAMEWORK:-}" "${CODEX_GPUI_SIGN_IDENTITY:--}"
    tools/mac.sh -l "$lane" "$update_env bash tools/bundle.sh --assemble /tmp/cgx-mac/release-$lane $mode $revision archive"
    destination=/tmp/cgx/release
    filename=codex-gpui-$version-macos-arm64.zip
    [[ $mode == normal ]] || { destination=/sdc-disk/tmp/cgx/qa-sync; filename=codex-gpui-qa.zip; }
    mkdir -p "$destination"
    scp -q -o BatchMode=yes -o ControlMaster=no -o ControlPath=none -o ConnectTimeout=15 "mac:/tmp/cgx-mac/release-$lane/bundle.zip" "$destination/$filename"
    printf '%s\n' "$destination/$filename"
    exit
fi
out=$2 mode=$3 revision=$4
[[ $(uname -s) == Darwin && $out == /tmp/* && ! -L $out ]] || { echo 'assembly requires an owned /tmp directory on macOS' >&2; exit 2; }
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/app/Cargo.toml)
name='Codex GPUI' identifier=dev.codexgpui.app
export CODEX_GPUI_REVISION=$revision
unset CODEX_GPUI_QA
if [[ $mode == qa ]]; then
    name='Codex GPUI QA' identifier=dev.codexgpui.qa
    export CODEX_GPUI_QA=1
elif [[ $mode != normal ]]; then
    exit 2
fi
cargo build --locked --release -p app
mkdir -p "$out"
stage=$(mktemp -d "$out/stage.XXXXXX")
app="$stage/$name.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$CARGO_TARGET_DIR/release/codex-gpui" "$app/Contents/MacOS/"
cp assets/app/AppIcon.icns "$app/Contents/Resources/"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDevelopmentRegion</key><string>en</string>
<key>CFBundleDisplayName</key><string>$name</string>
<key>CFBundleExecutable</key><string>codex-gpui</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleIdentifier</key><string>$identifier</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
<key>CFBundleName</key><string>$name</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CodexGPUIRevision</key><string>$revision</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
feed=${CODEX_GPUI_UPDATE_FEED:-}
key=${CODEX_GPUI_UPDATE_PUBLIC_KEY:-}
identity=${CODEX_GPUI_SIGN_IDENTITY:--}
sign_args=(--force --sign "$identity")
if [[ $identity == - ]]; then
    sign_args+=(--timestamp=none)
else
    sign_args+=(--timestamp --options runtime)
fi
if [[ -n $feed || -n $key ]]; then
    [[ -n $feed && -n $key && -d ${CODEX_GPUI_SPARKLE_FRAMEWORK:-} ]] || {
        echo 'updates require a feed, public key, and Sparkle.framework' >&2
        exit 2
    }
    [[ $feed == https://?* ]] || { echo 'the release update feed requires HTTPS' >&2; exit 2; }
    key_bytes=$(printf '%s' "$key" | base64 -D | wc -c)
    [[ $key_bytes -eq 32 ]] || { echo 'the update public key must contain 32 bytes' >&2; exit 2; }
    codesign --verify --deep --strict "$CODEX_GPUI_SPARKLE_FRAMEWORK"
    mkdir -p "$app/Contents/Frameworks"
    ditto "$CODEX_GPUI_SPARKLE_FRAMEWORK" "$app/Contents/Frameworks/Sparkle.framework"
    framework="$app/Contents/Frameworks/Sparkle.framework"
    for helper in Autoupdate Updater.app XPCServices/Downloader.xpc XPCServices/Installer.xpc; do
        codesign "${sign_args[@]}" --preserve-metadata=entitlements "$framework/Versions/Current/$helper"
    done
    codesign "${sign_args[@]}" "$framework"
    plutil -insert SUFeedURL -string "$feed" "$app/Contents/Info.plist"
    plutil -insert SUPublicEDKey -string "$key" "$app/Contents/Info.plist"
    plutil -insert SUEnableAutomaticChecks -bool YES "$app/Contents/Info.plist"
fi
plutil -lint "$app/Contents/Info.plist"
codesign "${sign_args[@]}" "$app"
codesign --verify --strict --verbose=2 "$app"
"$app/Contents/MacOS/codex-gpui" --qa-build-info
if [[ ${5:-} == archive ]]; then
    (cd "$stage" && ditto -c -k --keepParent "$name.app" "$out/bundle.zip")
    rm -rf -- "$stage"
else
    printf 'verified bundle: %s\n' "$app"
fi
