#!/bin/sh
# Build the WEC7-C2 bare-metal boot stub (M4 proof-of-life).
#
# Produces:
#   build/bootstub.elf  — linked ELF (for inspection / gdb)
#   build/bootstub.bin  — raw binary (what a bootloader would load)
#   build/bootstub.lst  — disassembly, for auditing the emitted ARMv7-A code
#
# Reproducible per Rule 7. Toolchain: arm-none-eabi (bare-metal).
# NOTE: building != booting. This validates the code assembles/links to real
# ARMv7-A. On-device execution is gated on HW map §6/§13 (Rules 2, 4, 8).
set -eu

CROSS=${CROSS:-arm-none-eabi-}
CC="${CROSS}gcc"
OBJCOPY="${CROSS}objcopy"
OBJDUMP="${CROSS}objdump"

HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/build"
mkdir -p "$OUT"

CFLAGS="-mcpu=cortex-a7 -marm -nostdlib -ffreestanding -Wall -Wextra"

echo "[1/3] assemble + link"
"$CC" $CFLAGS -T "$HERE/bootstub.ld" -Wl,--build-id=none \
      -o "$OUT/bootstub.elf" "$HERE/start.S"

echo "[2/3] objcopy -> raw binary"
"$OBJCOPY" -O binary "$OUT/bootstub.elf" "$OUT/bootstub.bin"

echo "[3/3] disassemble -> listing"
"$OBJDUMP" -d "$OUT/bootstub.elf" > "$OUT/bootstub.lst"

echo "OK:"
ls -l "$OUT"/bootstub.elf "$OUT"/bootstub.bin
echo "entry point:"
"${CROSS}readelf" -h "$OUT/bootstub.elf" | grep -i entry
