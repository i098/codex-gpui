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
    tools/mac.sh -l "$lane" "bash tools/bundle.sh --assemble /tmp/cgx-mac/release-$lane $mode $revision"
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
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --verbose=2 "$app"
"$app/Contents/MacOS/codex-gpui" --qa-build-info
(cd "$stage" && ditto -c -k --keepParent "$name.app" "$out/bundle.zip")
printf 'verified bundle: %s\n' "$app"
