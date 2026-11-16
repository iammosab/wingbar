#!/bin/bash
# wingbar catalog widget: adb
#
# Shows the Android devices adb can see (phones, tablets, emulators): the
# first one's model and how many more there are ("Pixel 7", "Pixel 7 +1").
# Ready devices come first.
#
# Levels (line 1 is "level|label"):
#   ok     every device is ready
#   warn   a device is connected but not usable yet (unauthorized: accept the
#          prompt on the phone; offline; recovery; ...)
#   (empty line: no device, or adb is not installed; the widget hides)
#
# Menu: one line per device (model and serial, or serial and state), then
# "Restart adb server".
#
# Needs: adb (Android platform-tools), found on PATH, else in
# $ANDROID_HOME/platform-tools, $ANDROID_SDK_ROOT/platform-tools or
# ~/Library/Android/sdk/platform-tools. Note that `adb devices` starts the adb
# server when it isn't running. No network, no secrets.
# Settings (environment, optional):
#   WINGBAR_ADB   the adb binary to use
# Suggested interval: 10s.

ADB=${WINGBAR_ADB:-}
if [ -z "$ADB" ]; then
	command -v adb >/dev/null 2>&1 && ADB=adb
	for d in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Library/Android/sdk"; do
		[ -n "$ADB" ] && break
		[ -n "$d" ] && [ -x "$d/platform-tools/adb" ] && ADB="$d/platform-tools/adb"
	done
fi
if [ -z "$ADB" ] || ! command -v "$ADB" >/dev/null 2>&1; then
	echo ""
	exit 0
fi

# "serial<TAB>state<TAB>name" per device, ready ones first. The name is the
# model with "_" as spaces, or the serial when adb doesn't say. These go into
# menu titles, so "|" (it would end the title and start a command) and
# control characters are taken out of everything adb says first.
DEVICES=$("$ADB" devices -l 2>/dev/null | tr -d '|' | tr -d '\001-\010\013-\037\177' | awk '
	NF < 2 || /^List of devices/ || /^\*/ { next }
	{
		model = ""
		for (i = 3; i <= NF; i++) if ($i ~ /^model:/) { model = substr($i, 7); gsub(/_/, " ", model) }
		state = $2
		# "no permissions (...)" and other states with spaces
		if (state == "no") state = "no permissions"
		line = $1 "\t" state "\t" (model != "" ? model : $1)
		if (state == "device") ready[++r] = line; else other[++o] = line
	}
	END {
		for (i = 1; i <= r; i++) print ready[i]
		for (i = 1; i <= o; i++) print other[i]
	}')

if [ -z "$DEVICES" ]; then
	echo ""
	exit 0
fi

COUNT=$(printf '%s\n' "$DEVICES" | awk 'END { print NR }')
FIRST=$(printf '%s\n' "$DEVICES" | awk -F '\t' 'NR == 1 { print $3 }')
NOT_READY=$(printf '%s\n' "$DEVICES" | awk -F '\t' '$2 != "device" { n++ } END { print n + 0 }')

LEVEL=ok
[ "$NOT_READY" -gt 0 ] && LEVEL=warn
LABEL=$FIRST
[ "$COUNT" -gt 1 ] && LABEL="$FIRST +$((COUNT - 1))"

echo "$LEVEL|$LABEL"
printf '%s\n' "$DEVICES" | awk -F '\t' '{
	if ($2 == "device") print "menu: " $3 " · " $1
	else print "menu: " $1 " · " $2
}'
echo "menu: ---"
printf 'menu: Restart adb server | %q kill-server; %q start-server\n' "$ADB" "$ADB"
