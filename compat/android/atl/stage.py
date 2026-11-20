#!/usr/bin/env python3
"""Stage ATL as the private prefix /opt/atl.

    stage.py SYSROOT BUILD ATL_SRC OUT ARCAN_ROOT ART_LIB

OUT/opt/atl gets the framework (api-impl.jar, framework-res.apk), the JNI
library and atl-run with their library closure (ART's own libraries come
from /opt/art, Arcan's shmif from /usr/lib), and the DejaVu fonts with a
fontconfig configuration.
"""
import os
import shutil
import subprocess
import sys

sysroot, build, src, out_root, arcan, art_lib = sys.argv[1:7]
out = os.path.join(out_root, "opt/atl")
shutil.rmtree(out_root, ignore_errors=True)
dirs = [f"{sysroot}/usr/lib/art", f"{sysroot}/usr/lib", f"{sysroot}/lib"]
# Already in /opt/art (ART's stage, ART_LIB) or /usr/lib (Arcan, Mesa).
art = set(os.listdir(art_lib))
SYSTEM = {"libarcan_shmif.so.0.16", "libEGL.so.1", "libGLESv2.so.2", "libdrm.so.2", "libgbm.so.1"}


def needed(p):
    r = subprocess.run(["llvm-readelf", "-d", p], capture_output=True, text=True).stdout
    return [l.split("[")[1].split("]")[0] for l in r.splitlines() if "(NEEDED)" in l]


def copy(srcf, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(os.path.realpath(srcf), dst)
    subprocess.run(["llvm-strip", "--strip-unneeded", dst], stderr=subprocess.DEVNULL)


copy(f"{build}/libtranslation_layer_main.so", f"{out}/lib/libtranslation_layer_main.so")
copy(f"{build}/atl-run", f"{out}/bin/atl-run")
todo, seen = [f"{build}/libtranslation_layer_main.so", f"{build}/atl-run"], {"libtranslation_layer_main.so"}
while todo:
    p = todo.pop()
    for lib in needed(p):
        if lib in seen or lib in SYSTEM or lib in art:
            continue
        seen.add(lib)
        f = next((f"{d}/{lib}" for d in dirs if os.path.exists(f"{d}/{lib}")), None)
        if not f:
            sys.exit(f"stage.py: {lib} (needed by {p}) not found")
        copy(f, f"{out}/lib/{lib}")
        todo.append(f)
shutil.copy2(f"{build}/api-impl.jar", f"{out}/api-impl.jar")
shutil.copy2(f"{src}/res/framework-res/framework-res.apk", f"{out}/framework-res.apk")
os.makedirs(f"{out}/fonts")
for f in ["DejaVuSans.ttf", "DejaVuSans-Bold.ttf", "DejaVuSans-Oblique.ttf", "DejaVuSans-BoldOblique.ttf",
          "DejaVuSansMono.ttf", "DejaVuSerif.ttf"]:
    shutil.copy2(f"{sysroot}/usr/share/fonts/dejavu/{f}", f"{out}/fonts/{f}")
with open(f"{out}/fonts.conf", "w") as fh:
    fh.write('<?xml version="1.0"?>\n<!DOCTYPE fontconfig SYSTEM "fonts.dtd">\n<fontconfig>\n'
             '  <dir>/opt/atl/fonts</dir>\n  <cachedir>/tmp/fontconfig</cachedir>\n'
             '  <alias><family>sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>\n'
             '  <alias><family>monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias>\n'
             '</fontconfig>\n')
print(f"stage.py: {len(seen)} libraries")
