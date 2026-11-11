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
echo "--- end of demo ---"
