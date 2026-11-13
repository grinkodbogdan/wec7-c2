#!/bin/sh
#
# Build the DragonFly/arm64 kernel and optionally boot it in QEMU.
#
# Usage: ./build.sh [build|ramdisk|mi5|run|clean]
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
DEJAVU_VERSION=${DEJAVU_VERSION:-2.37}
INTER_VERSION=${INTER_VERSION:-4.0}
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
MD_SRCS="in_cksum_md.c busdma_machdep.c locore.S exception.S swtch.S support.S
	machdep.c mp_machdep.c pmap.c trap.c sig_machdep.c vm_machdep.c elf_machdep.c
	intr_machdep.c
	gic.c clock.c
	cons_pl011.c fbcons.c qcom_msm8996.c qcom_ufs.c qcom_pmic_keys.c qcom_spmi.c qcom_qup_i2c.c qcom_touch.c qcom_pmi8994.c qcom_i2c9.c qcom_gpu.c qcom_scm.c qcom_rpm.c fbmem.c autoconf.c fdt_subr.c identcpu.c stubs.c selftest.c
	udivti3.c"

# "optional" files: what standard files depend on unconditionally, plus
# the kernel configuration proper: a memory disk and the ISO 9660
# filesystem, so there is something to mount as root, and tmpfs for
# somewhere to write.
# Kernel options beyond "standard": IPv4 and IPv6, Ethernet framing, 802.11 and
# its WPA ciphers and rate control, bpf, the loopback interface.
KERNEL_OPTIONS=${KERNEL_OPTIONS-"inet inet6 ether wlan wlan_ccmp wlan_tkip wlan_amrr bpf loop"}

MI_EXTRA_SRCS="kern/subr_diskmbr.c kern/subr_diskgpt.c
	dev/drm/msm/adreno_a5xx_dfly.c dev/drm/msm/msm_dfly.c
	dev/qcom/pcie_msm8996.c
	crypto/rijndael/rijndael-alg-fst.c crypto/rijndael/rijndael-api-fst.c
	crypto/rijndael/rijndael-api.c
	dev/disk/md/md.c dev/disk/vn/vn.c kern/tty_pty.c
	dev/misc/evdev/cdev.c dev/misc/evdev/evdev.c dev/misc/evdev/evdev_mt.c dev/misc/evdev/evdev_utils.c
	vfs/isofs/cd9660/cd9660_bmap.c vfs/isofs/cd9660/cd9660_lookup.c
	vfs/isofs/cd9660/cd9660_node.c vfs/isofs/cd9660/cd9660_rrip.c
	vfs/isofs/cd9660/cd9660_util.c vfs/isofs/cd9660/cd9660_vfsops.c
	vfs/isofs/cd9660/cd9660_vnops.c
	vfs/tmpfs/tmpfs_fifoops.c vfs/tmpfs/tmpfs_subr.c
	vfs/tmpfs/tmpfs_vfsops.c vfs/tmpfs/tmpfs_vnops.c
	vfs/ext2fs/ext2_alloc.c vfs/ext2fs/ext2_balloc.c vfs/ext2fs/ext2_bmap.c
	vfs/ext2fs/ext2_csum.c vfs/ext2fs/ext2_extents.c vfs/ext2fs/ext2_hash.c
	vfs/ext2fs/ext2_htree.c vfs/ext2fs/ext2_ihash.c vfs/ext2fs/ext2_inode.c
	vfs/ext2fs/ext2_inode_cnv.c vfs/ext2fs/ext2_lookup.c
	vfs/ext2fs/ext2_subr.c vfs/ext2fs/ext2_vfsops.c vfs/ext2fs/ext2_vnops.c
	vfs/msdosfs/msdosfs_conv.c vfs/msdosfs/msdosfs_denode.c vfs/msdosfs/msdosfs_fat.c
	vfs/msdosfs/msdosfs_lookup.c vfs/msdosfs/msdosfs_vfsops.c vfs/msdosfs/msdosfs_vnops.c
	vfs/hammer2/hammer2_admin.c vfs/hammer2/hammer2_inode.c vfs/hammer2/hammer2_subr.c vfs/hammer2/hammer2_bulkfree.c vfs/hammer2/hammer2_io.c vfs/hammer2/hammer2_synchro.c vfs/hammer2/hammer2_iocom.c vfs/hammer2/hammer2_vfsops.c vfs/hammer2/hammer2_chain.c vfs/hammer2/hammer2_ioctl.c vfs/hammer2/hammer2_vnops.c vfs/hammer2/hammer2_cluster.c vfs/hammer2/hammer2_lz4.c vfs/hammer2/hammer2_xops.c vfs/hammer2/hammer2_flush.c vfs/hammer2/hammer2_msgops.c vfs/hammer2/hammer2_freemap.c vfs/hammer2/hammer2_strategy.c vfs/hammer2/hammer2_ondisk.c vfs/hammer2/xxhash/xxhash.c vfs/hammer2/zlib/hammer2_zlib_adler32.c vfs/hammer2/zlib/hammer2_zlib_inflate.c vfs/hammer2/zlib/hammer2_zlib_deflate.c vfs/hammer2/zlib/hammer2_zlib_zutil.c vfs/hammer2/zlib/hammer2_zlib_inftrees.c vfs/hammer2/zlib/hammer2_zlib_inffast.c vfs/hammer2/zlib/hammer2_zlib_trees.c"

fetch() {
	if [ ! -d "$DFLY_SRC/sys" ]; then
		mkdir -p "$(dirname "$DFLY_SRC")"
		git clone --filter=blob:none "$DFLY_REPO" "$DFLY_SRC"
		git -C "$DFLY_SRC" checkout -q "$DFLY_COMMIT"
	fi
	# Overlay the port.  Plain copy keeps the tree usable with diff -ruN.
	cp -R "$HERE/sys/." "$S/"
	for d in lib libexec contrib bin sbin usr.bin; do
		[ -d "$HERE/$d" ] && cp -R "$HERE/$d/." "$DFLY_SRC/$d/"
	done
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
	# The kernel configuration: what "standard" brings in, plus the
	# options in KERNEL_OPTIONS (networking for Wi-Fi).  Each option
	# turns on its "optional" files in conf/files, its device count in
	# use_<option>.h and, for the protocols, its opt_<option>.h.
	for o in $KERNEL_OPTIONS; do
		O=$(echo "$o" | tr a-z A-Z)
		echo "#define N$O 1" > "$OBJ/opt/use_$o.h"
		[ -f "$OBJ/opt/opt_$o.h" ] && echo "#define $O 1" > "$OBJ/opt/opt_$o.h"
	done
	# net80211's debug messages, switched on per interface with
	# sysctl net.wlan.N.debug
	echo "#define IEEE80211_DEBUG 1" >> "$OBJ/opt/opt_wlan.h"
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

# PANIC_REBOOT_WAIT_TIME: a panic stays on screen for 90 s, not 15.
CFLAGS="--target=aarch64-unknown-none-elf -D__DragonFly__ -D_KERNEL \
	-DPANIC_REBOOT_WAIT_TIME=90 \
	-DHAVE_KERNEL_OPTION_HEADERS -include opt_global.h \
	-O2 -g -std=gnu99 -ffreestanding -fno-pic -fno-common \
	-mgeneral-regs-only -mstrict-align -fno-strict-aliasing \
	-fno-strict-overflow -fno-stack-protector -fno-omit-frame-pointer \
	-fno-asynchronous-unwind-tables -ferror-limit=10 \
	-nostdinc -I$OBJ/opt -I$OBJ/include -I$S $EXTRA_CFLAGS"
# The shared kernel is written for GCC and is not warning-clean under
# clang; the port's own code is held to -Wall -Werror.
MI_CFLAGS="$CFLAGS -w"
# The QCA6174 Wi-Fi driver: athp, Adrian Chadd's FreeBSD port of ath10k
# (ISC), with a shim header for what DragonFly spells differently.
ATHP_CFLAGS="-I$S/dev/netif/athp/dfly -I$S/dev/netif/athp $MI_CFLAGS \
	-include athp_dfly.h -DATHP_DFLY"
ATHP_SRCS=${ATHP_SRCS-"athp_idr.c if_athp_bmi.c if_athp_buf.c if_athp_core.c
	if_athp_debug.c if_athp_debug_stats.c if_athp_desc.c if_athp_dfly.c
	if_athp_fwlog.c if_athp_hal_compl.c if_athp_htc.c if_athp_htt.c
	if_athp_htt_rx.c if_athp_htt_tx.c if_athp_mac.c if_athp_main.c
	if_athp_pci.c if_athp_pci_ce.c if_athp_pci_chip.c if_athp_pci_config.c
	if_athp_pci_hif.c if_athp_pci_pipe.c if_athp_regio.c if_athp_regs.c
	if_athp_spectral.c if_athp_swap.c if_athp_taskq.c if_athp_thermal.c
	if_athp_trace.c if_athp_txrx.c if_athp_wmi.c if_athp_wmi_tlv.c
	regd/regd.c"}
# Its firmware, from linux-firmware (redistributable, LICENCE.atheros_firmware),
# linked into the kernel; there is no firmware loader from disk.
ATHP_FW_URL=https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/plain/ath10k/QCA6174/hw3.0
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
	    "$OBJ"/opt/opt_*.h; echo "$CFLAGS") | cksum)
	if [ "$new" != "$(cat "$OBJ/stamp" 2>/dev/null)" ]; then
		echo "$new" > "$OBJ/stamp"
	fi

	# Offsets of structure members for the assembly sources.
	# (genassym.sh reads the values out of common symbols' sizes.)
	$CC $MD_CFLAGS -fcommon -c "$P/aarch64/genassym.c" -o "$OBJ/genassym.o"
	NM=$NM sh "$S/kern/genassym.sh" "$OBJ/genassym.o" > "$OBJ/opt/assym.s"

	awk '$2 == "standard" && $1 ~ /^[a-z].*\.c$/ { print $1 }' \
	    "$S/conf/files" > "$OBJ/mi.files"
	# optional files whose options are all configured (as config(8))
	awk -v opts=" $KERNEL_OPTIONS " '
	    $2 == "optional" && $1 ~ /^[a-z].*\.c$/ {
		ok = 1; n = 0
		for (i = 3; i <= NF; i++) {
			if ($i !~ /^[a-z0-9_]+$/) break
			n++
			if (index(opts, " " $i " ") == 0) { ok = 0; break }
		}
		if (ok && n > 0) print $1
	    }' "$S/conf/files" | sort -u >> "$OBJ/mi.files"
	for f in $MI_EXTRA_SRCS; do
		echo "$f"
	done >> "$OBJ/mi.files"
	for f in bus_if.c device_if.c cpu_if.c ioconf.c vers.c; do
		echo "$OBJ/opt/$f"
	done >> "$OBJ/mi.files"
	for f in $MD_SRCS; do
		echo "platform/arm64/aarch64/$f"
	done > "$OBJ/md.files"

	for f in $ATHP_SRCS; do
		echo "dev/netif/athp/$f"
	done > "$OBJ/athp.files"

	compile_list "$MI_CFLAGS" "$OBJ/mi.files"
	compile_list "$MD_CFLAGS" "$OBJ/md.files"
	compile_list "$ATHP_CFLAGS" "$OBJ/athp.files"

	objs=""
	for f in $(cat "$OBJ/md.files" "$OBJ/mi.files" "$OBJ/athp.files"); do
		o=$(echo "$f" | sed "s#^$OBJ/##; s#^$S/##" | tr / _)
		objs="$objs $OBJ/o/${o%.*}.o"
	done
	if [ -n "$ATHP_SRCS" ]; then
		fw=$HERE/work/firmware/athp
		mkdir -p "$fw"
		for f in firmware-4.bin board.bin; do
			[ -s "$fw/$f" ] || curl -sSfL -o "$fw/$f" "$ATHP_FW_URL/$f" ||
			    rm -f "$fw/$f"
		done
		if [ -s "$fw/firmware-4.bin" ] && [ -s "$fw/board.bin" ]; then
			# symbols _binary_athp_fw4_bin_start etc. (if_athp_dfly.c)
			mkdir -p "$OBJ/fw"
			cp "$fw/firmware-4.bin" "$OBJ/fw/athp_fw4.bin"
			cp "$fw/board.bin" "$OBJ/fw/athp_board.bin"
			for f in athp_fw4 athp_board; do
				(cd "$OBJ/fw" && $OBJCOPY -I binary \
				    -O elf64-littleaarch64 \
				    --rename-section .data=.rodata,alloc,load,readonly,data,contents \
				    $f.bin $f.o)
				objs="$objs $OBJ/fw/$f.o"
			done
		fi
	fi
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
	rm -rf "${OBJ:?}/root"
	mkdir -p "$root/sbin" "$root/bin" "$root/dev" "$root/tmp" "$root/etc" "$root/mnt" "$root/h2"
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
	# Programs built against the real C library by world.sh, if any.
	# DragonFly's sh then becomes /bin/sh and the test shell /bin/tsh;
	# the test init stays /sbin/init (the real one needs /etc/rc).
	for f in "$HERE"/work/wobj/hello/hello "$HERE"/work/wobj/hello/keymenu "$HERE"/work/wobj/hello/cxxtest "$HERE"/work/wobj/hello/gltest; do
		[ -x "$f" ] && cp "$f" "$root/bin/"
	done
	sr=$HERE/work/sysroot
	if [ -x "$sr/bin/sh" ]; then
		mv "$root/bin/sh" "$root/bin/tsh"
		mkdir -p "$root/usr/bin" "$root/usr/sbin"
		for d in bin sbin usr/bin usr/sbin; do
			for f in "$sr/$d"/*; do
				[ -f "$f" ] || continue
				n=${f##*/}
				# The real init needs /etc/rc; leave it out
				# until there is one (the ramdisk is near its
				# 16MB limit).
				[ "$d/$n" = sbin/init ] && continue
				cp "$f" "$root/$d/$n"
			done
		done
		cp "$HERE/user/demo.sh" "$root/etc/demo.sh"
		# Arcan (world.sh arcan): the engine, the terminal frameserver,
		# the console appl, builtin scripts and the default font.
		a=$HERE/work/wobj/arcan
		asrc=$HERE/work/arcan-0.7.0.1
		if [ -x "$a/arcan" ]; then
			cp "$a/arcan" "$a/arcan_frameserver" "$a/afsrv_terminal" \
			    "$root/usr/bin/"
			mkdir -p "$root/usr/share/arcan/appl" \
			    "$root/usr/share/arcan/resources/fonts"
			cp -r "$asrc/data/appl/console" "$HERE/arcan/appl/hello" \
			    "$HERE/arcan/appl/term" "$HERE/arcan/appl/phone" \
			    "$root/usr/share/arcan/appl/"
			cp -r "$asrc/data/scripts" "$root/usr/share/arcan/scripts"
			cp "$asrc/data/resources/fonts/default.ttf" \
			    "$root/usr/share/arcan/resources/fonts/"
			# DejaVu for the phone shell (Sans, Bold, ExtraLight, Mono)
			fd=$HERE/work/fonts/dejavu-fonts-ttf-$DEJAVU_VERSION/ttf
			if [ ! -d "$fd" ]; then
				mkdir -p "$HERE/work/fonts"
				curl -sSfL "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_$(echo $DEJAVU_VERSION | tr . _)/dejavu-fonts-ttf-$DEJAVU_VERSION.tar.bz2" |
				    tar xjf - -C "$HERE/work/fonts"
			fi
			for f in DejaVuSans DejaVuSans-Bold DejaVuSans-ExtraLight \
			    DejaVuSansMono; do
				cp "$fd/$f.ttf" "$root/usr/share/arcan/resources/fonts/"
			done
			# Inter (as QRT's shell uses), OFL-licensed
			iz=$HERE/work/fonts/Inter-$INTER_VERSION.zip
			[ -f "$iz" ] || curl -sSfL -o "$iz" \
			    "https://github.com/rsms/inter/releases/download/v$INTER_VERSION/Inter-$INTER_VERSION.zip"
			for f in Inter-Regular Inter-Medium Inter-SemiBold \
			    InterDisplay-Light; do
				unzip -p "$iz" "extras/ttf/$f.ttf" \
				    > "$root/usr/share/arcan/resources/fonts/$f.ttf"
			done
			unzip -p "$iz" LICENSE.txt \
			    > "$root/usr/share/arcan/resources/fonts/Inter-LICENSE.txt"
			cp "$HERE/user/arcan.sh" "$root/etc/arcan.sh"
			# Wi-Fi: the shell's helper, dhclient's script, and a
			# resolv.conf that lives on tmpfs
			cp "$HERE/user/wifi.sh" "$root/etc/wifi.sh"
			cp "$HERE/user/dhclient-script" "$root/sbin/dhclient-script"
			chmod 755 "$root/sbin/dhclient-script"
			ln -sf /tmp/var/resolv.conf "$root/etc/resolv.conf"
		fi
		cp "$HERE/user/h2test.sh" "$root/etc/h2test.sh"
		cp "$HERE/user/touchtest.sh" "$root/etc/touchtest.sh"
		cp "$HERE/user/gpufw.sh" "$root/etc/gpufw.sh"
		cp "$HERE/user/gputest.sh" "$root/etc/gputest.sh"
		[ -n "${ARCAN_AUTORUN:-}" ] &&
		    echo "$ARCAN_AUTORUN" > "$root/etc/arcan.autorun"
		llvm-strip "$root"/bin/* "$root"/sbin/* "$root"/usr/bin/* \
		    2>/dev/null || :
	fi
	ln -s tmp/var "$root/var"
	[ "${FASTBOOT:-0}" = 1 ] && touch "$root/etc/fastboot"
	echo "DragonFly/arm64 test ramdisk" > "$root/etc/motd"
	genisoimage -quiet -R -o "$OBJ/root.iso" "$root"
	# The phone's boot image carries it compressed (the kernel
	# inflates it), leaving room under the bootloader's limit.
	gzip -9 -n -c "$OBJ/root.iso" > "$OBJ/root.iso.gz"
	echo "built $OBJ/root.iso"
}

# A boot image for the Xiaomi Mi 5 (gemini), to run with
#	fastboot boot work/obj/dragonfly-mi5-boot.img
# which loads it into RAM for one boot and writes nothing to the phone.
#
# The phone's device trees are compiled from LineageOS's kernel sources
# (Xiaomi's board files a1-*.dts); the bootloader picks the one that
# matches the board and fills in memory, command line and ramdisk.
# This follows the recipe proven on a real Mi 5 by the rpiios project
# (mi5/dtb.sh, shim/build-mi5.sh).  Needs cpp and dtc.
#
# The phone image boots fast: a quiet kernel (every console line costs a
# full panel refresh) and a ramdisk whose /etc/fastboot makes init skip
# the self-tests and the demo and start Arcan.  MI5_DEBUG=1 instead
# boots verbosely (-v: memory layout, each early boot stage) and runs
# everything.  MI5_APPEND adds boot arguments.
MI5_KERNEL_REPO=https://github.com/LineageOS/android_kernel_xiaomi_msm8996
MI5_KERNEL_SHA=535079d2fa367e4a4ea2111e5903c62dced89cd2	# lineage-22.2

mi5() {
	[ -f "$OBJ/kernel.img" ] || build
	if [ "${MI5_DEBUG:-0}" = 1 ]; then
		mi5_args="-v ${MI5_APPEND:-}"
		FASTBOOT=0 ramdisk >/dev/null
	else
		mi5_args="${MI5_APPEND:-}"
		FASTBOOT=1 ramdisk >/dev/null
	fi
	[ -f "$OBJ/root.iso" ] || ramdisk
	K="$OBJ/mi5-lineage-kernel"
	if [ ! -d "$K/.git" ]; then
		git init -q "$K"
		git -C "$K" sparse-checkout set --no-cone \
		    /arch/arm64/boot/dts/ /arch/arm/boot/dts/qcom/ \
		    /include/dt-bindings/ /scripts/dtc/include-prefixes/
		git -C "$K" fetch -q --depth 1 --filter=blob:none \
		    "$MI5_KERNEL_REPO" "$MI5_KERNEL_SHA"
		git -C "$K" checkout -q FETCH_HEAD
	fi
	mkdir -p "$OBJ/mi5-dtbs"
	dtbargs=""
	for d in "$K"/arch/arm64/boot/dts/qcom/a1-*.dts; do
		n=$(basename "$d" .dts)
		(cd "$K" && cpp -nostdinc -undef -D__DTS__ \
		    -x assembler-with-cpp -I arch/arm64/boot/dts/qcom \
		    -I arch/arm64/boot/dts -I include "$d" |
		    dtc -q -I dts -O dtb -o "$OBJ/mi5-dtbs/$n.dtb" -)
		dtbargs="$dtbargs --dtb $OBJ/mi5-dtbs/$n.dtb"
	done
	python3 "$HERE/tools/mkbootimg.py" --kernel "$OBJ/kernel.img" \
	    --ramdisk "$OBJ/root.iso.gz" $dtbargs \
	    --cmdline "vfs.root.mountfrom=cd9660:md0 $mi5_args" \
	    -o "$OBJ/dragonfly-mi5-boot.img"
}

run() {
	[ -f "$OBJ/kernel.img" ] || build
	[ -f "$OBJ/root.iso" ] || ramdisk
	exec qemu-system-aarch64 -M virt -cpu ${QEMU_CPU:-cortex-a72} \
	    -m ${QEMU_MEM:-1G} -smp ${QEMU_SMP:-1} -nographic -nic none \
	    -kernel "$OBJ/kernel.img" -initrd "${INITRD:-$OBJ/root.iso}" \
	    -append "vfs.root.mountfrom=cd9660:md0 $QEMU_APPEND" \
	    $QEMU_EXTRA
}

case "${1:-build}" in
fetch)	fetch ;;
build)	build; ramdisk ;;
ramdisk) ramdisk ;;
mi5)	mi5 ;;
run)	run ;;
clean)	rm -rf "$OBJ" ;;
*)	echo "usage: $0 [build|ramdisk|mi5|run|clean]" >&2; exit 1 ;;
esac
