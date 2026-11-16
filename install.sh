#!/bin/bash
# Install wingbar and switch over from Übersicht + simple-bar.
#
#   curl -fsSL https://raw.githubusercontent.com/iammosab/wingbar/main/install.sh | bash
#       downloads the latest release (no Rust needed)
#   ./install.sh               in a clone: builds from source
#   ./install.sh --download    in a clone: installs the latest release instead
#
# Safe to run again (also to update). Undo everything with uninstall.sh.
set -euo pipefail

REPO="iammosab/wingbar"

# Wait for every bar running from $1 to end (one started by hand, or left
# behind, isn't stopped by bootout); force it after 3 seconds.
stop_running() {
  local i
  pkill -TERM -f "^$1( |$)" 2>/dev/null || return 0
  for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
    pgrep -f "^$1( |$)" >/dev/null || return 0
    sleep 0.2
  done
  pkill -KILL -f "^$1( |$)" 2>/dev/null || true
  sleep 0.3
}

main() {
  local mode=auto
  case "${1:-}" in
    --download) mode=download ;;
    --source) mode=source ;;
    "") ;;
    *) echo "usage: install.sh [--download | --source]" >&2; exit 2 ;;
  esac

  # A clone is the folder this script sits in, if it holds wingbar's sources.
  # Piped into bash there is no such folder.
  local src="" here
  if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    if [ -f "$here/src/main.rs" ] && grep -q '^name = "wingbar"' "$here/Cargo.toml" 2>/dev/null; then
      src=$here
    fi
  fi
  if [ "$mode" = auto ]; then
    if [ -n "$src" ]; then mode=source; else mode=download; fi
  fi
  if [ "$mode" = source ] && [ -z "$src" ]; then
    echo "--source needs a clone: git clone https://github.com/$REPO.git" >&2
    exit 1
  fi

  local uid_n app agents parked plist sbs stage
  uid_n=$(id -u)
  app="$HOME/Applications/Wingbar.app"
  agents="$HOME/Library/LaunchAgents"
  parked="$agents/parked-by-wingbar"
  plist="$agents/local.wingbar.plist"
  sbs="$agents/com.jeantinland.simple-bar-server.plist"
  stage=$(mktemp -d)
  # expanded now: the trap runs after main returns, when $stage is gone
  # shellcheck disable=SC2064
  trap "rm -rf '$stage'" EXIT

  if [ "$mode" = source ]; then
    command -v cargo >/dev/null || {
      echo "Building from source needs Rust (brew install rust)," >&2
      echo "or install the ready-built app: ./install.sh --download" >&2
      exit 1
    }
    echo "→ building (release)"
    (cd "$src" && cargo build --release)
    local version
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$src/Cargo.toml" | head -1)
    "$src/scripts/bundle.sh" "$src/target/release/wingbar" "$version" "$stage"
  else
    echo "→ downloading the latest release"
    curl -fsSL "https://github.com/$REPO/releases/latest/download/wingbar-macos.zip" -o "$stage/wingbar.zip"
    ditto -x -k "$stage/wingbar.zip" "$stage"
    [ -x "$stage/Wingbar.app/Contents/MacOS/wingbar" ] || { echo "the download has no Wingbar.app" >&2; exit 1; }
    # curl doesn't quarantine, but a copy that came through a browser would be
    xattr -dr com.apple.quarantine "$stage/Wingbar.app" 2>/dev/null || true
  fi

  # Stop a running bar first: replacing a running program's file in place makes
  # macOS kill it for an invalid code signature.
  launchctl bootout "gui/$uid_n" "$plist" >/dev/null 2>&1 || true
  stop_running "$app/Contents/MacOS/wingbar"

  echo "→ installing $app"
  mkdir -p "$HOME/Applications"
  rm -rf "$app"
  ditto "$stage/Wingbar.app" "$app"
  mkdir -p "$HOME/.local/bin"
  ln -sf "$app/Contents/MacOS/wingbar" "$HOME/.local/bin/wingbar"

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
  if [ -f "$sbs" ]; then
    launchctl bootout "gui/$uid_n" "$sbs" >/dev/null 2>&1 || true
    mkdir -p "$parked" && mv "$sbs" "$parked/"
  fi

  echo "→ starting wingbar at login"
  mkdir -p "$agents"
  cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>local.wingbar</string>
  <key>ProgramArguments</key><array><string>$app/Contents/MacOS/wingbar</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardErrorPath</key><string>/tmp/wingbar.log</string>
</dict></plist>
EOF
  launchctl bootstrap "gui/$uid_n" "$plist"

  echo
  echo "Done. wingbar $("$app/Contents/MacOS/wingbar" --version 2>/dev/null | awk '{print $NF}') is running and starts at login."
  echo "If macOS asks, allow Wingbar in Privacy & Security → Accessibility"
  echo "(it reads the focused window's title directly instead of asking AeroSpace)."
  case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo "The wingbar command is in ~/.local/bin, which isn't on your PATH." ;;
  esac
}

# Everything is in main so a partly downloaded script never runs.
main "$@"
