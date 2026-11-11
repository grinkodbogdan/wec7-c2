#!/bin/sh
# gpufw.sh: look for the Adreno 530's signed zap shader (a530_zap.*) on
# the phone's partitions, all mounted read-only: FAT (the modem/firmware
# partition, under image/) and ext4 (system or vendor, under
# etc/firmware, vendor/firmware or firmware).  What it finds is copied
# to /tmp/fw for the GPU driver.
mkdir -p /tmp/fw /mnt
found=0
look() {	# device, filesystem
	for f in /mnt/image/a530* /mnt/IMAGE/A530* /mnt/a530* \
	    /mnt/etc/firmware/a530* /mnt/vendor/firmware/a530* \
	    /mnt/firmware/a530* /mnt/system/etc/firmware/a530*; do
		[ -f "$f" ] || continue
		echo "$1 ($2): $f $(wc -c < "$f") bytes"
		cp "$f" /tmp/fw/
		found=1
	done
}
for d in /dev/ufs*s*; do
	[ -c "$d" ] || continue
	if mount_msdos -o ro "$d" /mnt 2>/dev/null; then
		look "$d" fat; umount /mnt
	elif mount_ext2fs -o ro "$d" /mnt 2>/dev/null; then
		look "$d" ext4; umount /mnt
	fi
done
[ $found = 1 ] || echo "no a530 firmware found"
ls -l /tmp/fw
