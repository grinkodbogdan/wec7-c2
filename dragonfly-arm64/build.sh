#!/bin/sh
#
# Build the DragonFly/arm64 bring-up kernel and optionally boot it in QEMU.
#
# Usage: ./build.sh [build|run|clean]
#
# Needs: git, clang, ld.lld, llvm-objcopy; qemu-system-aarch64 for "run".
#
# The DragonFly source tree is not vendored.  It is fetched at a pinned
# commit into $DFLY_SRC (default: ./work/dragonfly) and the files under
# ./sys are overlaid on top of it, so ./sys contains only what this port
# adds.
#
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
DFLY_REPO=${DFLY_REPO:-https://github.com/DragonFlyBSD/DragonFlyBSD.git}
DFLY_COMMIT=${DFLY_COMMIT:-94448f622a06cde90ba42d688e9cab2448ad9ab3}
DFLY_SRC=${DFLY_SRC:-$HERE/work/dragonfly}
OBJ=${OBJ:-$HERE/work/obj}
CC=${CC:-clang}
LD=${LD:-ld.lld}
OBJCOPY=${OBJCOPY:-llvm-objcopy}
command -v "$OBJCOPY" >/dev/null 2>&1 || OBJCOPY=$(ls /usr/bin/llvm-objcopy* | head -1)

S=$DFLY_SRC/sys
P=$S/platform/arm64

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

# Mimic what config(8) + kern.fwd.mk do: <machine/*.h> resolves to the
# platform's headers, falling back to the cpu's, and <cpu/*.h> to the
# cpu's.
mkinclude() {
	rm -rf "$OBJ/include"
	mkdir -p "$OBJ/include/machine"
	ln -s "$S/cpu/aarch64/include" "$OBJ/include/cpu"
	for h in "$S"/cpu/aarch64/include/*.h "$P"/include/*.h; do
		ln -sf "$h" "$OBJ/include/machine/$(basename "$h")"
	done
}

CFLAGS="--target=aarch64-unknown-none-elf -D__DragonFly__ -D_KERNEL \
	-O2 -g -std=gnu99 -ffreestanding -fno-builtin -fno-pic \
	-mgeneral-regs-only -mstrict-align -fno-strict-aliasing \
	-fno-stack-protector -fno-omit-frame-pointer \
	-fno-asynchronous-unwind-tables \
	-Wall -Wextra -Wno-unused-parameter -Werror \
	-nostdinc -I$OBJ/include -I$S"

build() {
	fetch
	mkinclude
	objs=""
	for src in locore.S exception.S support.S machdep.c trap.c fdt_subr.c \
	    early_console.c gic.c clock.c identcpu.c; do
		o="$OBJ/${src%.*}.o"
		$CC $CFLAGS -c "$P/aarch64/$src" -o "$o"
		objs="$objs $o"
	done
	$LD -nostdlib -static -T "$P/conf/ldscript.aarch64" \
	    -o "$OBJ/kernel.elf" $objs
	$OBJCOPY -O binary "$OBJ/kernel.elf" "$OBJ/kernel.img"
	echo "built $OBJ/kernel.img ($(wc -c < "$OBJ/kernel.img") bytes)"
}

run() {
	[ -f "$OBJ/kernel.img" ] || build
	exec qemu-system-aarch64 -M virt -cpu ${QEMU_CPU:-cortex-a72} \
	    -m ${QEMU_MEM:-1G} -smp ${QEMU_SMP:-4} -nographic -nic none \
	    -kernel "$OBJ/kernel.img" ${QEMU_APPEND:+-append "$QEMU_APPEND"} \
	    $QEMU_EXTRA
}

case "${1:-build}" in
build)	build ;;
run)	run ;;
clean)	rm -rf "$OBJ" ;;
*)	echo "usage: $0 [build|run|clean]" >&2; exit 1 ;;
esac
