#!/bin/sh
#
# Build the DragonFly/arm64 kernel and optionally boot it in QEMU.
#
# Usage: ./build.sh [build|ramdisk|run|clean]
#
# Needs: git, clang, ld.lld, llvm-objcopy, llvm-nm, awk, python3,
# genisoimage;
# qemu-system-aarch64 for "run".
#
# The DragonFly source tree is not vendored.  It is fetched at a pinned
# commit into $DFLY_SRC (default: ./work/dragonfly) and the files under
# ./sys are overlaid on top of it, so ./sys contains only what this port
# adds.
#
# This script stands in for config(8) and the kernel makefiles, neither
# of which know about arm64 yet: it generates the option headers and
# source files config(8) would, then compiles every "standard" file from
# sys/conf/files plus the arm64 platform sources.
#
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
DFLY_REPO=${DFLY_REPO:-https://github.com/DragonFlyBSD/DragonFlyBSD.git}
DFLY_COMMIT=${DFLY_COMMIT:-94448f622a06cde90ba42d688e9cab2448ad9ab3}
DFLY_SRC=${DFLY_SRC:-$HERE/work/dragonfly}
OBJ=${OBJ:-$HERE/work/obj}
CC=${CC:-clang}
LD=${LD:-ld.lld}
NM=${NM:-llvm-nm}
OBJCOPY=${OBJCOPY:-llvm-objcopy}
JOBS=${JOBS:-$(nproc 2>/dev/null || echo 2)}
KERN_IDENT=${KERN_IDENT:-ARM64_GENERIC}

S=$DFLY_SRC/sys
P=$S/platform/arm64

# Platform sources, in link order (locore must come first).
MD_SRCS="locore.S exception.S swtch.S support.S
	machdep.c pmap.c trap.c sig_machdep.c vm_machdep.c elf_machdep.c
	intr_machdep.c
	gic.c clock.c
	cons_pl011.c fbcons.c autoconf.c fdt_subr.c identcpu.c stubs.c selftest.c
	udivti3.c"

# "optional" files: what standard files depend on unconditionally, plus
# the kernel configuration proper: a memory disk and the ISO 9660
# filesystem, so there is something to mount as root, and tmpfs for
# somewhere to write.
MI_EXTRA_SRCS="kern/subr_diskmbr.c kern/subr_diskgpt.c
	dev/disk/md/md.c
	vfs/isofs/cd9660/cd9660_bmap.c vfs/isofs/cd9660/cd9660_lookup.c
	vfs/isofs/cd9660/cd9660_node.c vfs/isofs/cd9660/cd9660_rrip.c
	vfs/isofs/cd9660/cd9660_util.c vfs/isofs/cd9660/cd9660_vfsops.c
	vfs/isofs/cd9660/cd9660_vnops.c
	vfs/tmpfs/tmpfs_fifoops.c vfs/tmpfs/tmpfs_subr.c
	vfs/tmpfs/tmpfs_vfsops.c vfs/tmpfs/tmpfs_vnops.c"

fetch() {
	if [ ! -d "$DFLY_SRC/sys" ]; then
		mkdir -p "$(dirname "$DFLY_SRC")"
		git clone --filter=blob:none "$DFLY_REPO" "$DFLY_SRC"
		git -C "$DFLY_SRC" checkout -q "$DFLY_COMMIT"
	fi
	# Overlay the port.  Plain copy keeps the tree usable with diff -ruN.
	cp -R "$HERE/sys/." "$S/"
	# The few machine-independent files that test for __x86_64__ need
	# to learn about __aarch64__.  Skip if already applied.
	for p in "$HERE"/patches/*.diff; do
		if git -C "$DFLY_SRC" apply --check "$p" 2>/dev/null; then
			git -C "$DFLY_SRC" apply "$p"
		fi
	done
}

# What config(8) does: <machine/*.h> resolves to the platform's headers,
# falling back to the cpu's; <cpu/*.h> to the cpu's; plus the opt_*.h,
# use_*.h, *_if.[ch], ioconf.c and vers.c files.
configure() {
	rm -rf "$OBJ/include" "$OBJ/opt"
	mkdir -p "$OBJ/include/machine" "$OBJ/opt" "$OBJ/o"
	ln -s "$S/cpu/aarch64/include" "$OBJ/include/cpu"
	for h in "$S"/cpu/aarch64/include/*.h "$P"/include/*.h; do
		ln -sf "$h" "$OBJ/include/machine/$(basename "$h")"
	done

	# Empty option headers select every option's default; a device
	# count of zero means "driver not configured".
	grep -rhoE '#include[[:space:]]+"opt_[a-zA-Z0-9_]+\.h"' "$S" |
	    sed -E 's/.*"(.*)"/\1/' | sort -u |
	    while read -r h; do : > "$OBJ/opt/$h"; done
	grep -rhoE '#include[[:space:]]+"use_[a-zA-Z0-9_]+\.h"' "$S" |
	    sed -E 's/.*"(.*)"/\1/' | sort -u | while read -r h; do
		n=$(echo "${h#use_}" | sed 's/\.h$//' | tr a-z A-Z)
		echo "#define N$n 0" > "$OBJ/opt/$h"
	done
	# The kernel configuration.  Kept minimal until there is a
	# userland to exercise more: no networking protocols, no
	# filesystems beyond what "standard" brings in.
	cat > "$OBJ/opt/opt_global.h" <<-EOG
	#define INVARIANTS 1
	EOG
	echo "#define MAXUSERS 0" > "$OBJ/opt/opt_maxusers.h"

	for m in kern/bus_if.m kern/device_if.m kern/cpu_if.m; do
		(cd "$OBJ/opt" &&
		    awk -f "$S/tools/makeobjops.awk" -- -h "$S/$m" &&
		    awk -f "$S/tools/makeobjops.awk" -- -c "$S/$m")
	done

	cat > "$OBJ/opt/ioconf.c" <<-EOG
	/* No statically configured devices; arm64 discovers them via FDT. */
	#include <sys/param.h>
	#include <sys/bus.h>
	#include <sys/bus_private.h>
	struct config_device config_devtab[] = { { 0 } };
	int devtab_count = 0;
	EOG

	# The framebuffer console's font: DragonFly's own cp437 8x16
	# console font, shipped uuencoded, as a C array.
	python3 - "$DFLY_SRC/share/syscons/fonts/cp437-8x16.fnt" \
	    > "$OBJ/opt/fbcons_font.h" <<-'EOG'
	import sys
	data = bytearray()
	for line in open(sys.argv[1], "rb").read().split(b"\n")[1:]:
	    line = line.rstrip(b"\r")
	    if line in (b"", b"end", b"`"):
	        continue
	    count = (line[0] - 32) & 63
	    v = [(c - 32) & 63 for c in line[1:]] + [0, 0, 0, 0]
	    out = bytearray()
	    for i in range(0, len(v) - 3, 4):
	        out += bytes(((v[i] << 2 | v[i + 1] >> 4) & 255,
	                      (v[i + 1] << 4 | v[i + 2] >> 2) & 255,
	                      (v[i + 2] << 6 | v[i + 3]) & 255))
	    data += out[:count]
	assert len(data) == 256 * 16, len(data)
	print("/* generated from share/syscons/fonts/cp437-8x16.fnt */")
	print("static const unsigned char fbcons_font[256 * 16] = {")
	for i in range(0, len(data), 16):
	    print("\t" + " ".join("0x%02x," % b for b in data[i:i + 16]))
	print("};")
	EOG

	reldate=$(awk '/^#define[[:space:]]+__DragonFly_version/ { print $3 }' \
	    "$S/sys/param.h")
	rev=$(git -C "$DFLY_SRC" describe --always 2>/dev/null || echo unknown)
	cat > "$OBJ/opt/vers.c" <<-EOG
	char version[] = "DragonFly/arm64 $rev #0: $(date)\\n";
	char ostype[] = "DragonFly";
	char osrelease[] = "$rev";
	int osreldate = $reldate;
	char kern_ident[] = "$KERN_IDENT";
	EOG
}

CFLAGS="--target=aarch64-unknown-none-elf -D__DragonFly__ -D_KERNEL \
	-DHAVE_KERNEL_OPTION_HEADERS -include opt_global.h \
	-O2 -g -std=gnu99 -ffreestanding -fno-pic -fno-common \
	-mgeneral-regs-only -mstrict-align -fno-strict-aliasing \
	-fno-strict-overflow -fno-stack-protector -fno-omit-frame-pointer \
	-fno-asynchronous-unwind-tables -ferror-limit=10 \
	-nostdinc -I$OBJ/opt -I$OBJ/include -I$S $EXTRA_CFLAGS"
# The shared kernel is written for GCC and is not warning-clean under
# clang; the port's own code is held to -Wall -Werror.
MI_CFLAGS="$CFLAGS -w"
MD_CFLAGS="$CFLAGS -Wall -Wextra -Wno-unused-parameter -Wno-sign-compare \
	-Wno-missing-field-initializers -Wno-pointer-sign -Werror"

# Compile "$1" (path relative to $S or absolute) unless its object is
# newer than both it and the stamp that tracks headers and flags.
compile_list() {
	flags=$1
	list=$2
	export CC flags S OBJ
	xargs -P "$JOBS" -I{} sh -c '
		src={}
		case "$src" in /*) ;; *) src="$S/$src" ;; esac
		o="$OBJ/o/$(echo "{}" | sed "s#^$OBJ/##; s#^$S/##" | tr / _)"
		o="${o%.*}.o"
		if [ ! -f "$o" ] || [ "$src" -nt "$o" ] ||
		    [ "$OBJ/stamp" -nt "$o" ]; then
			$CC $flags -c "$src" -o "$o" || exit 255
		fi' < "$list"
}

build() {
	fetch
	configure

	# Anything that can change what a compile produces, other than
	# the source file itself.
	new=$( (cat "$HERE"/sys/cpu/aarch64/include/*.h \
	    "$HERE"/sys/platform/arm64/include/*.h "$HERE"/patches/*.diff \
	    "$OBJ/opt/opt_global.h"; echo "$CFLAGS") | cksum)
	if [ "$new" != "$(cat "$OBJ/stamp" 2>/dev/null)" ]; then
		echo "$new" > "$OBJ/stamp"
	fi

	# Offsets of structure members for the assembly sources.
	# (genassym.sh reads the values out of common symbols' sizes.)
	$CC $MD_CFLAGS -fcommon -c "$P/aarch64/genassym.c" -o "$OBJ/genassym.o"
	NM=$NM sh "$S/kern/genassym.sh" "$OBJ/genassym.o" > "$OBJ/opt/assym.s"

	awk '$2 == "standard" && $1 ~ /^[a-z].*\.c$/ { print $1 }' \
	    "$S/conf/files" > "$OBJ/mi.files"
	for f in $MI_EXTRA_SRCS; do
		echo "$f"
	done >> "$OBJ/mi.files"
	for f in bus_if.c device_if.c cpu_if.c ioconf.c vers.c; do
		echo "$OBJ/opt/$f"
	done >> "$OBJ/mi.files"
	for f in $MD_SRCS; do
		echo "platform/arm64/aarch64/$f"
	done > "$OBJ/md.files"

	compile_list "$MI_CFLAGS" "$OBJ/mi.files"
	compile_list "$MD_CFLAGS" "$OBJ/md.files"

	objs=""
	for f in $(cat "$OBJ/md.files" "$OBJ/mi.files"); do
		o=$(echo "$f" | sed "s#^$OBJ/##; s#^$S/##" | tr / _)
		objs="$objs $OBJ/o/${o%.*}.o"
	done
	$LD -nostdlib -static -T "$P/conf/ldscript.aarch64" \
	    -o "$OBJ/kernel.elf" $objs
	$OBJCOPY -O binary "$OBJ/kernel.elf" "$OBJ/kernel.img"
	echo "built $OBJ/kernel.img ($(wc -c < "$OBJ/kernel.img") bytes)"
}

# A root filesystem with the test programs under user/: an init that
# checks the kernel and a small shell.  There is no libc or cross
# toolchain for DragonFly/arm64 yet, so they are freestanding and link
# against the mini C library in user/libc.c.
ramdisk() {
	root="${OBJ:?}/root"
	mkdir -p "$root/sbin" "$root/bin" "$root/dev" "$root/tmp" "$root/etc"
	ucflags="--target=aarch64-unknown-none-elf -O2 -ffreestanding
	    -fno-pic -fno-stack-protector -fno-builtin
	    -fno-asynchronous-unwind-tables -Wall -Wextra -Werror -nostdinc"
	$CC $ucflags -c "$HERE/user/libc.c" -o "$OBJ/ulibc.o"
	for prog in sbin/init bin/sh; do
		name=$(basename $prog)
		$CC $ucflags -c "$HERE/user/$name.c" -o "$OBJ/u$name.o"
		$LD -nostdlib -static -e _start -Ttext=0x400000 \
		    -o "$root/$prog" "$OBJ/u$name.o" "$OBJ/ulibc.o"
	done
	echo "DragonFly/arm64 test ramdisk" > "$root/etc/motd"
	genisoimage -quiet -R -o "$OBJ/root.iso" "$root"
	echo "built $OBJ/root.iso"
}

run() {
	[ -f "$OBJ/kernel.img" ] || build
	[ -f "$OBJ/root.iso" ] || ramdisk
	exec qemu-system-aarch64 -M virt -cpu ${QEMU_CPU:-cortex-a72} \
	    -m ${QEMU_MEM:-1G} -smp ${QEMU_SMP:-1} -nographic -nic none \
	    -kernel "$OBJ/kernel.img" -initrd "$OBJ/root.iso" \
	    -append "vfs.root.mountfrom=cd9660:md0 $QEMU_APPEND" \
	    $QEMU_EXTRA
}

case "${1:-build}" in
build)	build; ramdisk ;;
ramdisk) ramdisk ;;
run)	run ;;
clean)	rm -rf "$OBJ" ;;
*)	echo "usage: $0 [build|ramdisk|run|clean]" >&2; exit 1 ;;
esac
