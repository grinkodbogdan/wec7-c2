#!/bin/sh
# touchtest.sh: tap the on-screen keyboard through machdep.touch_inject
# (QEMU, booted with touch.fake=1): types "ls", then Enter.
# Panel 1080x1920, canvas half that; keyboard rows are 44 canvas pixels
# from y = 676, ten keys of 54 across.
tap() {	# canvas x, y
	sleep 1
	sysctl -q machdep.touch_inject=$(( ($1 * 2) * 65536 + $2 * 2 )) >/dev/null
}
tap 459 786	# l (row 3, key 9)
tap 81 786	# s (row 3, key 2)
tap 486 874	# enter (row 5)
