#!/bin/bash
# Remove wingbar and bring back Übersicht + simple-bar exactly as before.
#   ./uninstall.sh
#   curl -fsSL https://raw.githubusercontent.com/iammosab/wingbar/main/uninstall.sh | bash
# (Installed with Homebrew? Use: brew services stop wingbar && brew uninstall wingbar)
set -uo pipefail

main() {
  local uid_n agents parked plist
  uid_n=$(id -u)
  agents="$HOME/Library/LaunchAgents"
  parked="$agents/parked-by-wingbar"
  plist="$agents/local.wingbar.plist"

  launchctl bootout "gui/$uid_n" "$plist" >/dev/null 2>&1
  # a bar started by hand isn't stopped by bootout
  pkill -TERM -f "^$HOME/Applications/Wingbar.app/Contents/MacOS/wingbar( |$)" 2>/dev/null
  rm -f "$plist" "$HOME/.local/bin/wingbar"
  rm -rf "$HOME/Applications/Wingbar.app"

  if [ -f "$parked/com.jeantinland.simple-bar-server.plist" ]; then
    mv "$parked/com.jeantinland.simple-bar-server.plist" "$agents/"
    launchctl bootstrap "gui/$uid_n" "$agents/com.jeantinland.simple-bar-server.plist" >/dev/null 2>&1
    rmdir "$parked" 2>/dev/null
  fi
  if [ -d "/Applications/Übersicht.app" ]; then
    osascript -e 'tell application "System Events" to make login item at end with properties {path:"/Applications/Übersicht.app", hidden:false}' >/dev/null 2>&1
    open -a "Übersicht"
  fi
  echo "wingbar removed; Übersicht + simple-bar are back."
}

main "$@"
