#!/bin/sh
# compat/linux/linuxdrv/build.sh OUT SYSROOT - the Linux driver host.
#
# Linux as a library (LKL, github.com/lkl/linux) at the commit QRT's ports use,
# with QRT's patches (lkl/qrt.patch: device trees, the GIC, non-coherent DMA,
# SMC calls, and its fixes to Qualcomm drivers for the Xiaomi Mi A1) and
# Flare's (lkl/flare.patch: interrupts and SMC calls through the host, as a
# user process cannot own either), configured with QRT's arm64 config plus
# lkl/config-flare, then linuxdrv.c linked with it into one static musl
# program: OUT/linuxdrv. About 25 minutes on 4 cores the first time.
#
# Needs: git, aarch64-linux-gnu-gcc and binutils, flex, bison, bc, clang, lld;
# SYSROOT: an aarch64 musl sysroot with libgcc (compat/android/atl's).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
sys=$2
COMMIT=d0f76a77e8c2ca68bd7d96190b8da09040eab072
X=aarch64-linux-gnu-
src="$out/lkl-src"
if [ ! -d "$src/.git" ]; then
    git init -q "$src"
    git -C "$src" remote add origin https://github.com/lkl/linux.git
fi
if [ "$(git -C "$src" rev-parse HEAD 2>/dev/null)" != "$COMMIT" ]; then
    git -C "$src" fetch -q --depth 1 origin "$COMMIT"
    git -C "$src" -c advice.detachedHead=false checkout -q FETCH_HEAD
fi
cd "$src"
# The patches, once: a rebuild then only compiles what changed. When a patch
# changes, start again from the pristine tree.
want=$(cat "$here/lkl/qrt.patch" "$here/lkl/flare.patch" | sha256sum | cut -d' ' -f1)
if [ "$(cat .flare-patched 2>/dev/null)" != "$want" ]; then
    git reset -q --hard "$COMMIT"
    git clean -qfdx
    git apply "$here/lkl/qrt.patch"
    git apply "$here/lkl/flare.patch"
    echo "$want" > .flare-patched
fi
echo "linuxdrv/build.sh: configuring LKL"
make ARCH=lkl CROSS_COMPILE=$X defconfig >/dev/null
# tools/lkl rewrites .config the first time it runs (as QRT's ports/lkl.sh
# notes): let it, then put QRT's and Flare's options on top
make -C tools/lkl CROSS_COMPILE=$X "$src/.config" >/dev/null 2>&1 || true
# The phone's driver families from QRT's config for the Mi A1
# (lkl/config-phone: GPU, Wi-Fi and networking, audio, battery, backlight,
# vibrator, sensors, the partitions' file systems; extracted from
# lkl/config-qrt, QRT's whole config, whose core drivers - interrupt
# controller, timers, CPU frequency, resets - stalled the phone at boot);
# lkl/config-arm64 and lkl/config-flare on top, checked below.
cat "$here/lkl/config-phone" "$here/lkl/config-arm64" "$here/lkl/config-flare" >> .config
make ARCH=lkl CROSS_COMPILE=$X olddefconfig >/dev/null 2>&1
# Every option asked for must have taken: Kconfig silently drops what it
# cannot set (an option with no prompt, an unmet dependency).
bad=0
for f in "$here/lkl/config-arm64" "$here/lkl/config-flare"; do
    for kv in $(grep -E '^CONFIG_[A-Za-z0-9_]+=' "$f"); do
        k=${kv%%=*}; v=${kv#*=}
        got=$(grep -E "^$k=" .config | cut -d= -f2)
        [ "$v" = n ] && [ -z "$got" ] && continue
        if [ "$got" != "$v" ]; then
            echo "linuxdrv/build.sh: $k: asked for $v, Kconfig made it ${got:-unset}" >&2
            bad=1
        fi
    done
done
[ $bad = 0 ] || exit 1
echo "linuxdrv/build.sh: building LKL (Linux $(make -s kernelversion))"
make -C tools/lkl CROSS_COMPILE=$X -j"$(nproc)" "$src/tools/lkl/lib/lkl.o" >"$out/lkl-build.log" 2>&1 \
    || { tail -30 "$out/lkl-build.log"; exit 1; }
${X}objcopy --strip-debug tools/lkl/lib/lkl.o "$out/lkl.o"
gcclib=$(dirname "$(ls "$sys"/usr/lib/gcc/aarch64-alpine-linux-musl/*/libgcc.a | head -1)")
# zlib, static (the root filesystem's pieces): Alpine's, pinned in
# alpine.lock next to this script
z="$out/zsys"
[ -f "$z/usr/lib/libz.a" ] || python3 -I "$here/../../arcan/alpine.py" "$out/zcache" "$z" "$here/alpine.lock" zlib-static zlib-dev
clang --target=aarch64-alpine-linux-musl --sysroot="$sys" --gcc-toolchain="$sys/usr" -fuse-ld=lld -static -O2 \
    -Wall -Wno-unused-function -I tools/lkl/include -I"$z/usr/include" -o "$out/linuxdrv" "$here/linuxdrv.c" "$out/lkl.o" \
    "$z/usr/lib/libz.a" -lpthread -L"$gcclib" -lgcc
llvm-strip "$out/linuxdrv"
echo "linuxdrv/build.sh: $out/linuxdrv ($(du -h "$out/linuxdrv" | cut -f1))"
