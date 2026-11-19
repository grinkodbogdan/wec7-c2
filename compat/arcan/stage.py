#!/usr/bin/env python3
"""Stage Arcan, Mesa and their runtime libraries as a root filesystem overlay.

    stage.py SYSROOT ARCAN_ROOT MESA_ROOT OUT [EXTRA_BIN...]

Libraries are found by following the ELF NEEDED entries of the programs
(and of Mesa's dlopened GBM backend) and stripped on the way.
"""
import os
import shutil
import subprocess
import sys

sysroot, aroot, mroot, out = sys.argv[1:5]
extra = sys.argv[5:]
shutil.rmtree(out, ignore_errors=True)
libdirs = [f"{aroot}/usr/lib", f"{mroot}/usr/lib", f"{sysroot}/usr/lib", f"{sysroot}/lib"]
# Provided by Flare's root filesystem (musl's loader).
SYSTEM = ("libc.musl-aarch64.so.1", "ld-musl-aarch64.so.1", "libc.so")


def needed(path):
    r = subprocess.run(["llvm-readelf", "-d", path], capture_output=True, text=True).stdout
    return [l.split("[")[1].split("]")[0] for l in r.splitlines() if "(NEEDED)" in l]


def find(lib):
    return next((f"{d}/{lib}" for d in libdirs if os.path.exists(f"{d}/{lib}")), None)


def copy(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(os.path.realpath(src), dst)
    subprocess.run(["llvm-strip", "--strip-unneeded", dst], stderr=subprocess.DEVNULL)


done = set()


def pull(path):
    for lib in needed(path):
        if lib in done or lib in SYSTEM:
            continue
        done.add(lib)
        src = find(lib)
        if not src:
            sys.exit(f"stage.py: {lib} (needed by {path}) not found")
        copy(src, f"{out}/usr/lib/{lib}")
        pull(src)


for b in ["arcan", "arcan_frameserver", "afsrv_avfeed", "arcan_db"]:
    copy(f"{aroot}/usr/bin/{b}", f"{out}/usr/bin/{b}")
    pull(f"{aroot}/usr/bin/{b}")
for b in extra:
    copy(b, f"{out}/usr/bin/{os.path.basename(b)}")
    pull(b)
copy(f"{mroot}/usr/lib/gbm/dri_gbm.so", f"{out}/usr/lib/gbm/dri_gbm.so")
pull(f"{mroot}/usr/lib/gbm/dri_gbm.so")
shutil.copytree(f"{aroot}/usr/share/arcan", f"{out}/usr/share/arcan")
print("stage.py: libraries:", " ".join(sorted(done)))
