#!/bin/sh
# tools/bootimg/early-boot.sh KERNEL-ELF [LOAD-ADDR [SECONDS]]
#
# Early boot of a real phone's kernel, on QEMU: a virt machine whose RAM
# (0x40000000..0x100000000) covers the Mi A1's, the raw Image loaded where
# Xiaomi's LK puts it (0x80008000: the kernel must move itself), started
# from CPU reset. After SECONDS, prints the panel's splash buffer
# (0x90001000, 1080x1920 RGB888, console scale 2) as text.
#
# It gets as far as the first access to the SoC's own devices (QEMU virt
# has none there: the GIC at 0x0b000000 faults). What it checks is everything
# before that: the move, the caches, the page tables, the heap, the console.
#
#   cargo xtask build --system system/xiaomi-tissot.boot.toml
#   tools/bootimg/early-boot.sh target/aarch64-unknown-none-softfloat/release/flare-kernel
set -eu
kernel=$1
addr=${2:-0x80008000}
wait=${3:-20}
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
llvm-objcopy -O binary "$kernel" "$tmp/Image"
qemu-system-aarch64 -M virt -cpu cortex-a53 -m 3G -smp 1 -display none -monitor none -serial none \
    -device loader,file="$tmp/Image",addr="$addr",cpu-num=0 -qmp unix:"$tmp/qmp",server=on,wait=off &
qemu=$!
sleep "$wait"
python3 -I - "$tmp" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1] + "/qmp")
f = s.makefile("rw")
f.readline()
for c in ({"execute": "qmp_capabilities"},
          {"execute": "pmemsave", "arguments": {"val": 0x90001000, "size": 1080 * 1920 * 3,
                                                "filename": sys.argv[1] + "/fb.raw"}}):
    f.write(json.dumps(c) + "\n")
    f.flush()
    while "return" not in (r := json.loads(f.readline())) and "error" not in r:
        pass
PY
kill "$qemu" 2>/dev/null || true
python3 -I "$root/tools/bootimg/fbtext.py" "$tmp/fb.raw" 1080 1920 3240 3 2 | grep -v '^[[:space:]]*$'
