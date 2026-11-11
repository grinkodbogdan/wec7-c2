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
fi
echo "--- end of demo ---"
