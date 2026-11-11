# HAMMER2 test, run by the boot demo and from keymenu: create a volume,
# fill it, remount it, compare every file.
# A 1.1GB sparse file in a tmpfs (the flash stays read-only).
echo "--- HAMMER2 ---"
if mount_tmpfs -s 1200m tmpfs /mnt 2>/dev/null &&
    dd if=/dev/zero of=/mnt/h2.img bs=1m seek=1099 count=1 2>/dev/null &&
    vnconfig vn1 /mnt/h2.img && newfs_hammer2 -L ROOT /dev/vn1 >/dev/null 2>&1 &&
    mount_hammer2 /dev/vn1@ROOT /h2 2>/dev/null; then
	mkdir /h2/bin && cp /bin/* /sbin/* /h2/bin/ &&
	    dd if=/dev/zero of=/h2/zeros bs=1m count=40 2>/dev/null
	sync
	df -h /h2 | tail -1
	umount /h2 && mount_hammer2 /dev/vn1@ROOT /h2 2>/dev/null
	n=0; t=0
	for f in /bin/* /sbin/*; do
		t=$((t+1)); cmp -s $f /h2/bin/${f##*/} || n=$((n+1))
	done
	echo "HAMMER2: $t files written, remounted, $n differ"
	umount /h2; vnconfig -u vn1; umount /mnt
else
	echo "HAMMER2: could not set up"
fi
