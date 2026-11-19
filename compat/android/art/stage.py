#!/usr/bin/env python3
"""Stage ART from an Alpine root as the private prefix /opt/art.

    stage.py ALPINE_ROOT OUT

OUT/opt/art gets dalvikvm, ART's libraries and JNI natives with their
library closure (EGL and libdrm come from Flare's Mesa in /usr/lib), the
private musl loader, the boot class path jars and ICU data.
"""
import os
import shutil
import subprocess
import sys

root, out_root = sys.argv[1:3]
out = os.path.join(out_root, "opt/art")
shutil.rmtree(out_root, ignore_errors=True)
natives = f"{root}/usr/lib/java/dex/art/natives"
dirs = [f"{root}/usr/lib/art", natives, f"{root}/usr/lib", f"{root}/lib"]
# Provided by Flare's own Mesa build in /usr/lib.
SYSTEM = {"libEGL.so.1", "libGLESv2.so.2", "libgbm.so.1", "libdrm.so.2"}


def needed(p):
    r = subprocess.run(["llvm-readelf", "-d", p], capture_output=True, text=True).stdout
    return [l.split("[")[1].split("]")[0] for l in r.splitlines() if "(NEEDED)" in l]


def find(lib):
    return next((f"{d}/{lib}" for d in dirs if os.path.exists(f"{d}/{lib}")), None)


def copy(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(os.path.realpath(src), dst)
    subprocess.run(["llvm-strip", "--strip-unneeded", dst], stderr=subprocess.DEVNULL)


roots = [f"{natives}/{x}" for x in sorted(os.listdir(natives))]
roots += [f"{root}/usr/lib/art/{x}" for x in sorted(os.listdir(f"{root}/usr/lib/art"))]
seen = set()
for p in roots:
    copy(p, f"{out}/lib/{os.path.basename(p)}")
    seen.add(os.path.basename(p))
copy(f"{root}/usr/bin/dalvikvm", f"{out}/bin/dalvikvm")
todo = [f"{root}/usr/bin/dalvikvm"] + roots
while todo:
    p = todo.pop()
    for lib in needed(p):
        if lib in seen or lib in SYSTEM or lib == "libc.musl-aarch64.so.1":
            continue
        seen.add(lib)
        src = find(lib)
        if not src:
            sys.exit(f"stage.py: {lib} (needed by {p}) not found")
        copy(src, f"{out}/lib/{lib}")
        todo.append(src)
copy(f"{root}/lib/ld-musl-aarch64.so.1", f"{out}/lib/ld-musl-aarch64.so.1")
os.symlink("ld-musl-aarch64.so.1", f"{out}/lib/libc.musl-aarch64.so.1")
os.makedirs(f"{out}/framework")
for j in sorted(os.listdir(f"{root}/usr/lib/java/dex/art")):
    if j.endswith(".jar"):
        shutil.copy2(f"{root}/usr/lib/java/dex/art/{j}", f"{out}/framework/{j}")
shutil.copytree(f"{root}/usr/share/icu", f"{out}/icu")
print(f"stage.py: {len(seen)} libraries")
