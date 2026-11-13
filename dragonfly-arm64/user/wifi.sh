#!/bin/sh
# Wi-Fi for the phone shell.
#   wifi.sh up            PCIe link, the athp driver, wlan0
#   wifi.sh scan          scan, then the results (ifconfig's list scan)
#   wifi.sh connect CONF  wpa_supplicant with CONF, then DHCP
#   wifi.sh status        ifconfig wlan0
#   wifi.sh down          stop wpa_supplicant and dhclient
PATH=/sbin:/bin:/usr/sbin:/usr/bin
IF=wlan0
mkdir -p /tmp/var/run /tmp/var/db

wait_for() {	# wait_for SECONDS COMMAND...
	n=$1; shift
	while [ "$n" -gt 0 ]; do
		"$@" && return 0
		sleep 1
		n=$((n - 1))
	done
	return 1
}

linked() { [ "$(sysctl -n hw.mi5.pcie_up 2>/dev/null)" = 1 ]; }
ready() { [ "$(sysctl -n hw.mi5.wlan_ready 2>/dev/null)" = 1 ]; }
# by the interface list: a plain "ifconfig wlan0" can fail on an
# 802.11 status query while wlan0 exists
have_if() {
	for i in $(ifconfig -l); do
		[ "$i" = "$IF" ] && return 0
	done
	return 1
}

stop() {	# stop NAME...: by process name (dhclient forks itself away)
	ps -axo pid,comm | while read -r p c; do
		for n in "$@"; do
			[ "$c" = "$n" ] && kill "$p" 2>/dev/null
		done
	done
}

up() {
	if ! linked; then
		sysctl -q hw.mi5.pcie_up=1 >/dev/null
		wait_for 10 linked || { echo "no PCIe link"; return 1; }
	fi
	if ! ready; then
		sysctl -q hw.mi5.wlan_attach=1 >/dev/null
		wait_for 60 ready || { echo "driver: $(sysctl -n hw.mi5.wlan)"; return 1; }
	fi
	if ! have_if; then
		ifconfig "$IF" create wlandev athp0 || return 1
	fi
	ifconfig "$IF" up
}

case "$1" in
up)
	up
	;;
scan)
	up || exit 1
	# scans, waits for the result and lists it (with -v, the IEs)
	ifconfig -v "$IF" scan 2>&1
	;;
connect)
	[ -r "$2" ] || { echo "usage: wifi.sh connect CONF"; exit 1; }
	up || exit 1
	stop dhclient wpa_supplicant
	sleep 1
	wpa_supplicant -B -D bsd -i "$IF" -c "$2" \
	    -P /tmp/var/run/wpa_supplicant.pid -f /tmp/var/wpa.log || exit 1
	# associated within 30 s, then an address
	n=30
	while [ $n -gt 0 ]; do
		case "$(ifconfig "$IF")" in
		*"status: associated"*) break ;;
		esac
		sleep 1
		n=$((n - 1))
	done
	[ $n -gt 0 ] || { echo "not associated"; exit 1; }
	dhclient "$IF" > /tmp/var/dhclient.log 2>&1 &
	;;
status)
	ifconfig "$IF" 2>/dev/null
	;;
down)
	stop dhclient wpa_supplicant
	ifconfig "$IF" down 2>/dev/null
	;;
*)
	echo "usage: wifi.sh up|scan|connect CONF|status|down"
	exit 1
	;;
esac
