#!/bin/bash
# wingbar catalog widget: aerospace-mode
#
# Shows AeroSpace's current binding mode ("resize", "service", ...) while it is
# not the default "main" mode, so you can see that keys do something else.
#
# Levels (line 1 is "level|label"):
#   ok     a mode other than main: its name
#   (empty line: the main mode, AeroSpace not running, or not installed; the
#   widget hides)
#
# Menu: "Back to main mode".
#
# Needs: the aerospace CLI (AeroSpace 0.15 or later for `list-modes`). No
# network, no secrets.
# Settings (environment, optional):
#   WINGBAR_AEROSPACE   the aerospace binary to use
# Suggested interval: 2s. To update at once, add this to aerospace.toml:
#   on-mode-changed = ['exec-and-forget wingbar refresh aerospace-mode']

AEROSPACE=${WINGBAR_AEROSPACE:-aerospace}
if ! command -v "$AEROSPACE" >/dev/null 2>&1; then
	echo ""
	exit 0
fi

MODE=$("$AEROSPACE" list-modes --current 2>/dev/null | head -n 1 | tr -d '|' | tr '[:cntrl:]' ' ')
MODE=$(printf '%s' "$MODE" | awk '{ $1 = $1; print }')

if [ -z "$MODE" ] || [ "$MODE" = "main" ]; then
	echo ""
	exit 0
fi

echo "ok|$MODE"
printf 'menu: Back to main mode | %q mode main\n' "$AEROSPACE"
