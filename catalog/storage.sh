#!/bin/bash
# wingbar catalog widget: storage
#
# Shows how full the startup disk is, as a percentage ("72%"). On APFS the
# system, data and other volumes share one container, so the percentage is
# (size - available) / size of the data volume, which is what Finder and
# System Settings count.
#
# Levels (line 1 is "level|label"):
#   ok     below the warning threshold
#   warn   at or above WINGBAR_STORAGE_WARN percent (default 85)
#   alert  at or above WINGBAR_STORAGE_ALERT percent (default 95)
#   (empty line: the volume could not be read; the widget hides)
#
# Menu: the free space, "Open Disk Utility" and "Open Storage Settings".
#
# Needs: df (part of macOS). No network, no secrets.
# Settings (environment, all optional):
#   WINGBAR_STORAGE_PATH   the volume to measure (default: /System/Volumes/Data,
#                          or / where that does not exist)
#   WINGBAR_STORAGE_WARN   warn threshold in percent (default 85)
#   WINGBAR_STORAGE_ALERT  alert threshold in percent (default 95)
#   WINGBAR_DF             the df command (default: df)
# Suggested interval: 1m.

export LC_ALL=C
DF=${WINGBAR_DF:-df}
WARN=${WINGBAR_STORAGE_WARN:-85}
ALERT=${WINGBAR_STORAGE_ALERT:-95}
case $WARN in '' | *[!0-9]*) WARN=85 ;; esac
case $ALERT in '' | *[!0-9]*) ALERT=95 ;; esac

VOLUME=${WINGBAR_STORAGE_PATH:-}
if [ -z "$VOLUME" ]; then
	VOLUME=/
	[ -d /System/Volumes/Data ] && VOLUME=/System/Volumes/Data
fi

# `df -k` prints a header, then: filesystem, size, used, available, capacity,
# ... in 1024-byte blocks. The filesystem name may contain spaces, so the
# capacity ("61%") is found first and the numbers are read before it.
read -r SIZE AVAIL <<EOF
$("$DF" -k "$VOLUME" 2>/dev/null | awk 'NR > 1 { for (i = 4; i <= NF; i++) if ($i ~ /^[0-9]+%$/) { s = $(i-3); a = $(i-1); break } } END { if (s != "") print s, a }')
EOF

case $SIZE in '' | *[!0-9]*) SIZE= ;; esac
case $AVAIL in '' | *[!0-9]*) AVAIL= ;; esac
if [ -z "$SIZE" ] || [ -z "$AVAIL" ] || [ "$SIZE" -eq 0 ] || [ "$AVAIL" -gt "$SIZE" ]; then
	echo ""
	exit 0
fi

# the used share, rounded to the nearest percent
PCT=$(((100 * (SIZE - AVAIL) + SIZE / 2) / SIZE))

LEVEL=ok
[ "$PCT" -ge "$WARN" ] && LEVEL=warn
[ "$PCT" -ge "$ALERT" ] && LEVEL=alert

echo "$LEVEL|$PCT%"
awk -v a="$AVAIL" -v s="$SIZE" 'BEGIN { printf "menu: %.1f GB free of %.1f GB\n", a * 1024 / 1e9, s * 1024 / 1e9 }'
echo "menu: ---"
echo "menu: Open Disk Utility | open -a 'Disk Utility'"
echo "menu: Open Storage Settings | open 'x-apple.systempreferences:com.apple.settings.Storage'"
