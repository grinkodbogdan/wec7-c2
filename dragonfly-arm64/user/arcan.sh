# Start Arcan with the console appl (a terminal running sh) on /dev/fb0.
# Rendering through OSMesa at half the panel resolution: on the Adreno
# (freedreno, /dev/dri/card0) when the GPU comes up, else in software.
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

# The GPU: its firmware from the phone's partitions, the zap shader, the
# start-up; /dev/dri/card0 appears when it runs (sysctl hw.mi5.gpu_ready).
# ARCAN_GPU=0 keeps software rendering.
if [ "${ARCAN_GPU:-1}" = 1 ] && sysctl -n hw.mi5.gpu_ready >/dev/null 2>&1; then
	if [ "$(sysctl -n hw.mi5.gpu_ready)" != 1 ]; then
		sh /etc/gpufw.sh >/dev/null 2>&1
		sysctl -q hw.mi5.gpu_zap=1 >/dev/null 2>&1
		n=0
		while [ $n -lt 30 ] && [ "$(sysctl -n hw.mi5.gpu_ready)" != 1 ]; do
			sleep 0.5 2>/dev/null || sleep 1
			n=$((n + 1))
		done
	fi
	if [ "$(sysctl -n hw.mi5.gpu_ready)" = 1 ]; then
		export OSMESA_DRM_DEVICE=/dev/dri/card0
		echo "arcan: rendering on the GPU" > /tmp/arcan/renderer
	else
		echo "arcan: GPU not ready, software rendering" > /tmp/arcan/renderer
	fi
fi
exec arcan -d /tmp/arcan/arcan.sqlite "${@:-term}"
