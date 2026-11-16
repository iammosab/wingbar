#!/bin/bash
# Wrap a wingbar binary in Wingbar.app and sign it ad hoc.
# Usage: scripts/bundle.sh <binary> <version> <output dir>   → <output dir>/Wingbar.app
# Used by install.sh (building from source) and the release workflow.
set -euo pipefail
BIN=$1
VERSION=$2
APP="$3/Wingbar.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/wingbar"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>local.wingbar</string>
  <key>CFBundleName</key><string>Wingbar</string>
  <key>CFBundleExecutable</key><string>wingbar</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
EOF
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
