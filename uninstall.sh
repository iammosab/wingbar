#!/bin/bash
# Remove wingbar and bring back Übersicht + simple-bar exactly as before.
set -uo pipefail
UID_N=$(id -u)
AGENTS="$HOME/Library/LaunchAgents"
PARKED="$AGENTS/parked-by-wingbar"
PLIST="$AGENTS/local.wingbar.plist"

launchctl bootout "gui/$UID_N" "$PLIST" >/dev/null 2>&1
rm -f "$PLIST" "$HOME/.local/bin/wingbar"
rm -rf "$HOME/Applications/Wingbar.app"

if [ -f "$PARKED/com.jeantinland.simple-bar-server.plist" ]; then
  mv "$PARKED/com.jeantinland.simple-bar-server.plist" "$AGENTS/"
  launchctl bootstrap "gui/$UID_N" "$AGENTS/com.jeantinland.simple-bar-server.plist" >/dev/null 2>&1
  rmdir "$PARKED" 2>/dev/null
fi
if [ -d "/Applications/Übersicht.app" ]; then
  osascript -e 'tell application "System Events" to make login item at end with properties {path:"/Applications/Übersicht.app", hidden:false}' >/dev/null 2>&1
  open -a "Übersicht"
fi
echo "wingbar removed; Übersicht + simple-bar are back."
