#!/usr/bin/env bash
# Builds `Codex GPUI.app` on the Mac (tools/mac.sh lane `bundle`), signs it ad hoc, checks it
# with `codesign -v` and `plutil -lint`, zips it and pulls the zip to /tmp/cgx/release/.
# It never launches the app. Usage: tools/bundle.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/app/Cargo.toml)
zip=codex-gpui-$version-macos-arm64.zip
tools/mac.sh -l bundle "bash -s -- $version $zip" <<'EOF'
set -euo pipefail
version=$1 zip=$2 out=/tmp/cgx-mac/release
app="$out/Codex GPUI.app"
cargo build --release -p app
rm -rf "$out" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$CARGO_TARGET_DIR/release/codex-gpui" "$app/Contents/MacOS/"
cp assets/app/AppIcon.icns "$app/Contents/Resources/"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Codex GPUI</string>
  <key>CFBundleExecutable</key><string>codex-gpui</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIdentifier</key><string>dev.codexgpui.app</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Codex GPUI</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --verbose=2 "$app"
cd "$out" && ditto -c -k --keepParent "Codex GPUI.app" "$zip"
EOF
mkdir -p /tmp/cgx/release
scp -q "mac:/tmp/cgx-mac/release/$zip" /tmp/cgx/release/
ls -l "/tmp/cgx/release/$zip"
