#!/bin/bash
# wingbar catalog widget: network
#
# Shows how this Mac is online: the Wi-Fi network's name, "Ethernet", or
# "Offline". It follows the default route, so with both Wi-Fi and a cable it
# names the one in use. Behind a VPN it names the connection underneath, or
# "VPN" when there is none to be found.
#
# The Wi-Fi name: recent macOS hides it from apps without Location permission
# (it reads "<redacted>"); the widget then shows "Wi-Fi".
#
# Levels (line 1 is "level|label"):
#   ok    Wi-Fi (or a VPN): the network name, or "Wi-Fi" when it is hidden
#   eth   a wired connection (Ethernet, USB or Thunderbolt adapter): "Ethernet"
#   dim   no default route: "Offline"
#
# Menu: the interface and its address, "Open Network Settings", and
# "Open Wi-Fi Settings" on Wi-Fi.
#
# Needs: route, networksetup and ipconfig (all part of macOS). It reads local
# state only: no network traffic, no secrets.
# Settings (environment, all optional, mainly for tests):
#   WINGBAR_ROUTE, WINGBAR_NETWORKSETUP, WINGBAR_IPCONFIG   the commands
# Suggested interval: 10s.

ROUTE=${WINGBAR_ROUTE:-route}
NETWORKSETUP=${WINGBAR_NETWORKSETUP:-networksetup}
IPCONFIG=${WINGBAR_IPCONFIG:-ipconfig}

NETWORK_SETTINGS="open 'x-apple.systempreferences:com.apple.Network-Settings.extension'"
WIFI_SETTINGS="open 'x-apple.systempreferences:com.apple.wifi-settings-extension'"

# Text for the label or a menu title: without "|" (in a menu line it would end
# the title and start a command) and control characters (a line break would
# start a line of its own).
plain() {
	printf '%s' "$1" | tr -d '|[:cntrl:]'
}

# The interface of the default route ("en0"), or nothing when offline.
IFACE=$(plain "$("$ROUTE" -n get default 2>/dev/null | awk '$1 == "interface:" { print $2; exit }')")

if [ -z "$IFACE" ]; then
	echo "dim|Offline"
	echo "menu: No default route"
	echo "menu: ---"
	echo "menu: Open Network Settings | $NETWORK_SETTINGS"
	exit 0
fi

# Hardware ports as "device<TAB>port name" lines ("en0	Wi-Fi").
# ("|" and control characters out: device and port names go into menu titles)
PORTS=$("$NETWORKSETUP" -listallhardwareports 2>/dev/null | tr -d '|' | tr -d '\001-\010\013-\037\177' | awk '
	/^Hardware Port: / { sub(/^Hardware Port: /, ""); port = $0 }
	/^Device: / { if (port != "") print $2 "\t" port; port = "" }')
WIFI_DEV=$(printf '%s\n' "$PORTS" | awk -F '\t' '$2 == "Wi-Fi" || $2 == "AirPort" { print $1; exit }')

is_port() {
	printf '%s\n' "$PORTS" | awk -F '\t' -v d="$1" '$1 == d { found = 1 } END { exit !found }'
}

addr() {
	"$IPCONFIG" getifaddr "$1" 2>/dev/null
}

# A VPN's tunnel (utun, ppp, ipsec) is not a hardware port: name the
# connection under it, the Wi-Fi first, else the first wired port with an
# address.
VIA=""
if ! is_port "$IFACE"; then
	VIA=$IFACE
	DEV=""
	if [ -n "$WIFI_DEV" ] && [ -n "$(addr "$WIFI_DEV")" ]; then
		DEV=$WIFI_DEV
	else
		for d in $(printf '%s\n' "$PORTS" | awk -F '\t' '{ print $1 }'); do
			if [ "$d" != "$WIFI_DEV" ] && [ -n "$(addr "$d")" ]; then
				DEV=$d
				break
			fi
		done
	fi
	if [ -z "$DEV" ]; then
		echo "ok|VPN"
		echo "menu: VPN: $IFACE"
		echo "menu: ---"
		echo "menu: Open Network Settings | $NETWORK_SETTINGS"
		exit 0
	fi
	IFACE=$DEV
fi

IP=$(plain "$(addr "$IFACE")")

if [ -n "$WIFI_DEV" ] && [ "$IFACE" = "$WIFI_DEV" ]; then
	# the least indented "SSID : " line: the interface's own, not a nested one
	SSID=$("$IPCONFIG" getsummary "$IFACE" 2>/dev/null | awk '
		/^[[:space:]]*SSID : / {
			match($0, /^[[:space:]]*/)
			if (!found || RLENGTH < depth) { found = 1; depth = RLENGTH; v = $0; sub(/^[[:space:]]*SSID : /, "", v) }
		}
		END { if (found) print v }')
	case $SSID in "<redacted>" | "<"*">") SSID="" ;; esac
	if [ -z "$SSID" ]; then
		# older macOS: "Current Wi-Fi Network: <name>"
		SSID=$("$NETWORKSETUP" -getairportnetwork "$IFACE" 2>/dev/null |
			awk '/^Current (Wi-Fi|AirPort) Network: / { sub(/^Current (Wi-Fi|AirPort) Network: /, ""); print; exit }')
	fi
	# what the network is called is not ours to trust (see plain)
	SSID=$(plain "$SSID")
	echo "ok|${SSID:-Wi-Fi}"
	echo "menu: Wi-Fi ($IFACE)${IP:+: $IP}"
	[ -n "$VIA" ] && echo "menu: VPN: $VIA"
	echo "menu: ---"
	echo "menu: Open Wi-Fi Settings | $WIFI_SETTINGS"
	echo "menu: Open Network Settings | $NETWORK_SETTINGS"
else
	PORT=$(printf '%s\n' "$PORTS" | awk -F '\t' -v d="$IFACE" '$1 == d { print $2; exit }')
	PORT=$(plain "$PORT")
	echo "eth|Ethernet"
	echo "menu: ${PORT:-Ethernet} ($IFACE)${IP:+: $IP}"
	[ -n "$VIA" ] && echo "menu: VPN: $VIA"
	echo "menu: ---"
	echo "menu: Open Network Settings | $NETWORK_SETTINGS"
fi
