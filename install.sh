#!/bin/bash
# Install wingbar and switch over from Übersicht + simple-bar.
# Safe to run again. Undo everything with ./uninstall.sh
set -euo pipefail
cd "$(dirname "$0")"
UID_N=$(id -u)
APP="$HOME/Applications/Wingbar.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
AGENTS="$HOME/Library/LaunchAgents"
PARKED="$AGENTS/parked-by-wingbar"
PLIST="$AGENTS/local.wingbar.plist"
SBS="$AGENTS/com.jeantinland.simple-bar-server.plist"

echo "→ building (release)"
cargo build --release

echo "→ installing $APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/wingbar "$APP/Contents/MacOS/wingbar"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>local.wingbar</string>
  <key>CFBundleName</key><string>Wingbar</string>
  <key>CFBundleExecutable</key><string>wingbar</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
EOF
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
mkdir -p "$HOME/.local/bin"
ln -sf "$APP/Contents/MacOS/wingbar" "$HOME/.local/bin/wingbar"

echo "→ stopping Übersicht (simple-bar) and simple-bar-server"
osascript -e 'tell application id "tracesOf.Uebersicht" to quit' >/dev/null 2>&1 || true
sleep 1
# if it didn't quit when asked (no Automation permission), close it directly
if pgrep -f "bersicht.app/Contents/MacOS/" >/dev/null; then
  pkill -TERM -f "bersicht.app/Contents/" || true
  sleep 1
  pkill -KILL -f "bersicht.app/Contents/" 2>/dev/null || true
fi
osascript -e 'tell application "System Events" to delete (every login item whose name is "Übersicht")' >/dev/null 2>&1 || true
if [ -f "$SBS" ]; then
  launchctl bootout "gui/$UID_N" "$SBS" >/dev/null 2>&1 || true
  mkdir -p "$PARKED" && mv "$SBS" "$PARKED/"
fi

echo "→ starting wingbar at login"
cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>local.wingbar</string>
  <key>ProgramArguments</key><array><string>$APP/Contents/MacOS/wingbar</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardErrorPath</key><string>/tmp/wingbar.log</string>
</dict></plist>
EOF
launchctl bootout "gui/$UID_N" "$PLIST" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$UID_N" "$PLIST"

echo
echo "Done. wingbar is running and starts at login."
echo "If macOS asks, allow Wingbar in Privacy & Security → Accessibility"
echo "(it reads the focused window's title directly instead of asking AeroSpace)."
