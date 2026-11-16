#!/bin/bash
# wingbar catalog widget: mic
#
# Shows whether the microphone (the default input device) is muted, as an
# icon: "mic" when it is live, "mic.slash" when its input volume is 0. It is
# meant to be icon-only (its catalog definition sets "iconOnly": true); the
# label, "On" or "Muted", shows when iconOnly is turned off.
#
# Levels (line 1 is "level|label"):
#   ok     the input volume is above 0: "On"
#   alert  the input volume is 0: "Muted"
#   dim    the input device has no volume control macOS can read (some USB
#          and Bluetooth devices), so muted or not can't be told: "No level"
#   (empty line: osascript failed; the widget hides)
# It also prints "icon: mic" or "icon: mic.slash".
#
# `mic.sh toggle` mutes the input, or unmutes it to the volume it had before
# (kept in $TMPDIR/wingbar-mic-volume; 75 when unknown). The catalog
# definition uses it as the click action. It prints nothing.
#
# Needs: osascript (part of macOS). No network, no secrets.
# Settings (environment, optional, mainly for tests):
#   WINGBAR_OSASCRIPT   the osascript command
# Suggested interval: 5s (a click refreshes it at once).

OSASCRIPT=${WINGBAR_OSASCRIPT:-osascript}
SAVED="${TMPDIR:-/tmp}/wingbar-mic-volume"

VOL=$("$OSASCRIPT" -e 'input volume of (get volume settings)' 2>/dev/null)
STATUS=$?
VOL=$(printf '%s' "$VOL" | tr -d '[:space:]')

if [ "${1:-}" = "toggle" ]; then
	case $VOL in '' | *[!0-9]*) exit 0 ;; esac
	if [ "$VOL" -gt 0 ]; then
		printf '%s\n' "$VOL" >"$SAVED" 2>/dev/null
		"$OSASCRIPT" -e 'set volume input volume 0' >/dev/null 2>&1
	else
		BACK=$(cat "$SAVED" 2>/dev/null | tr -d '[:space:]')
		case $BACK in '' | *[!0-9]* | 0) BACK=75 ;; esac
		"$OSASCRIPT" -e "set volume input volume $BACK" >/dev/null 2>&1
	fi
	exit 0
fi

if [ $STATUS -ne 0 ] || [ -z "$VOL" ]; then
	echo ""
	exit 0
fi

case $VOL in
*[!0-9]*)
	# "missing value": the device has no input volume
	echo "dim|No level"
	echo "icon: mic"
	;;
0)
	echo "alert|Muted"
	echo "icon: mic.slash"
	;;
*)
	echo "ok|On"
	echo "icon: mic"
	;;
esac
