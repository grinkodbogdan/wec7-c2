# Start Arcan with the console appl (a terminal running sh) on /dev/fb0.
# Software rendering (OSMesa) at half the panel resolution.
export ARCAN_RESOURCEPATH=/usr/share/arcan/resources
export ARCAN_FONTPATH=/usr/share/arcan/resources/fonts
export ARCAN_SCRIPTPATH=/usr/share/arcan/scripts
export ARCAN_APPLBASEPATH=/usr/share/arcan/appl
export ARCAN_BINPATH=/usr/bin/arcan_frameserver
export ARCAN_STATEPATH=/tmp/arcan
export ARCAN_APPLTEMPPATH=/tmp/arcan
export ARCAN_LOGPATH=/tmp/arcan/logs
export XDG_RUNTIME_DIR=/tmp/arcan
export HOME=/tmp
export SHELL=/bin/sh
export ARCAN_FRAMESERVER_STDERR=/tmp/arcan/logs/frameserver.log
mkdir -p /tmp/arcan/logs
exec arcan -d /tmp/arcan/arcan.sqlite "${@:-term}"
