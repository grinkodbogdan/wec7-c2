# Run by the test init at boot, before the interactive shell: a few of
# DragonFly's own programs, built against its real C library.
echo
echo "--- DragonFly userland on $(uname -m) ---"
uname -sr
echo "cpus: $(sysctl -n hw.ncpu)   memory: $(( $(sysctl -n hw.physmem) / 1048576 )) MB"
echo "programs: $(ls /bin /sbin /usr/bin | wc -l)"
df -h / /tmp
ps ax | tail -4
hello | tail -1
if [ -e /dev/ufs0 ]; then
	echo "--- phone storage (read-only) ---"
	echo "disks: $(cd /dev && ls -d ufs[0-9] | tr '\n' ' ')"
	echo "ufs0 slices: $(cd /dev && ls -d ufs0s* 2>/dev/null | wc -l)"
	printf 'LBA 1 of ufs0: '
	dd if=/dev/ufs0 bs=4096 skip=1 count=1 2>/dev/null | head -c 8
	echo
	t0=$(date +%s)
	dd if=/dev/ufs0 of=/dev/null bs=128k count=512 2>&1 | tail -1
	echo "64MB read in $(( $(date +%s) - t0 )) s"
	# Android's ext4 partitions, read-only: persist (GPT entry 11)
	# and cache (12).
	for p in 11:persist 12:cache; do
		n=${p%%:*}; name=${p#*:}
		if mount_ext2fs -o ro /dev/ufs0s$n /mnt 2>/dev/null; then
			echo "$name (ufs0s$n), ext4, mounted read-only:"
			df -h /mnt | tail -1
			ls /mnt | head -8 | tr '\n' ' '; echo
			umount /mnt
		else
			echo "$name (ufs0s$n): could not mount"
		fi
	done
fi
# HAMMER2 on a 1.1GB sparse file in a tmpfs (the flash stays read-only).
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
echo "--- end of demo ---"
