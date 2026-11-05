#!/usr/bin/env bash
#
# Build UEFI firmware for the Orange Pi Zero 2W (Allwinner H618).
#
# Produces out/opiz2w-uefi-sunxi-with-spl.bin:
#   [ U-Boot SPL (DRAM init) | FIT: TF-A BL31 + EDK2 UEFI + board DTB ]
# which is written to an SD card at the 8 KiB offset, exactly like a normal
# u-boot-sunxi-with-spl.bin.
#
# Usage: ./build.sh [DEBUG|RELEASE]     (default DEBUG: verbose serial log)
#
# Needs (Ubuntu/Debian): git make gcc gcc-aarch64-linux-gnu python3
#   python3-setuptools python3-pyelftools uuid-dev acpica-tools
#   device-tree-compiler bison flex libssl-dev bc swig libgnutls28-dev
#
# SPDX-License-Identifier: BSD-2-Clause-Patent

set -euo pipefail

TARGET="${1:-DEBUG}"
case "$TARGET" in DEBUG|RELEASE|NOOPT) ;; *) echo "Unknown target $TARGET" >&2; exit 1 ;; esac

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${WORK:-$HERE/work}"
OUT="$HERE/out"
JOBS="${JOBS:-$(nproc)}"
CROSS="${CROSS_COMPILE:-aarch64-linux-gnu-}"

# Pinned upstream versions this port was written against.
EDK2_TAG="${EDK2_TAG:-edk2-stable202608}"
TFA_TAG="${TFA_TAG:-v2.15.0}"
UBOOT_TAG="${UBOOT_TAG:-v2026.10}"

for tool in git make "${CROSS}gcc" iasl dtc python3; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done

mkdir -p "$WORK" "$OUT"

fetch() { # url tag dir
  if [ ! -d "$WORK/$3/.git" ]; then
    git clone --depth 1 -b "$2" "$1" "$WORK/$3"
  fi
}

echo "==> Fetching sources"
fetch https://github.com/tianocore/edk2.git "$EDK2_TAG" edk2
git -C "$WORK/edk2" submodule update --init --depth 1 \
  BaseTools/Source/C/BrotliCompress/brotli \
  MdeModulePkg/Library/BrotliCustomDecompressLib/brotli \
  MdePkg/Library/MipiSysTLib/mipisyst \
  MdePkg/Library/BaseFdtLib/libfdt
fetch https://github.com/ARM-software/arm-trusted-firmware.git "$TFA_TAG" tf-a
fetch https://github.com/u-boot/u-boot.git "$UBOOT_TAG" u-boot

echo "==> Building TF-A BL31 (sun50i_h616 also covers the H618)"
make -C "$WORK/tf-a" -j"$JOBS" CROSS_COMPILE="$CROSS" PLAT=sun50i_h616 DEBUG=0 bl31
BL31="$WORK/tf-a/build/sun50i_h616/release/bl31.bin"

echo "==> Building U-Boot (only SPL, mkimage and the board DTB are used)"
make -C "$WORK/u-boot" orangepi_zero2w_defconfig
make -C "$WORK/u-boot" -j"$JOBS" CROSS_COMPILE="$CROSS" BL31="$BL31" SCP=/dev/null
SPL="$WORK/u-boot/spl/sunxi-spl.bin"
MKIMAGE="$WORK/u-boot/tools/mkimage"
DTB="$WORK/u-boot/u-boot.dtb"

echo "==> Building EDK2 ($TARGET)"
(
  cd "$WORK/edk2"
  export WORKSPACE="$WORK/edk2-build"
  export PACKAGES_PATH="$WORK/edk2:$HERE"
  export GCC_AARCH64_PREFIX="$CROSS"
  mkdir -p "$WORKSPACE"
  make -C BaseTools -j"$JOBS"
  # edksetup.sh reads unset variables, so relax -u while sourcing it.
  set +u
  . ./edksetup.sh
  set -u
  build -n "$JOBS" -a AARCH64 -t GCC -b "$TARGET" \
    -p OrangePiZero2W/OrangePiZero2W.dsc
)
FD="$WORK/edk2-build/Build/OrangePiZero2W/${TARGET}_GCC/FV/OPIZ2W_EFI.fd"

echo "==> Packing FIT + SPL"
STAGE="$WORK/fit"
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp "$FD" "$STAGE/OPIZ2W_EFI.fd"
cp "$BL31" "$STAGE/bl31.bin"
cp "$DTB" "$STAGE/board.dtb"
cp "$HERE/boot/uefi.its" "$STAGE/uefi.its"
(cd "$STAGE" && "$MKIMAGE" -f uefi.its uefi.itb)

# The FIT goes right after SPL, at 32 KiB into the image or the next
# 512-byte boundary after SPL if SPL is bigger (SPL works this out itself).
SPL_SIZE=$(stat -c %s "$SPL")
PAD=$(( SPL_SIZE > 32768 ? (SPL_SIZE + 511) / 512 * 512 : 32768 ))
IMG="$OUT/opiz2w-uefi-sunxi-with-spl.bin"
{
  cat "$SPL"
  head -c $(( PAD - SPL_SIZE )) /dev/zero | tr '\0' '\377'
  cat "$STAGE/uefi.itb"
} > "$IMG"
cp "$FD" "$OUT/OPIZ2W_EFI.fd"

echo
echo "Done: $IMG ($(stat -c %s "$IMG") bytes)"
echo "Write it to a SPARE microSD card (this overwrites its first few MiB):"
echo "  sudo dd if=$IMG of=/dev/sdX bs=1024 seek=8 conv=fsync"
