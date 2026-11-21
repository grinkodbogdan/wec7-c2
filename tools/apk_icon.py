#!/usr/bin/env python3
"""apk_icon.py APK OUT.png [SIZE]: an app's launcher icon, for the phone
shell's launcher.

The largest PNG named like a launcher icon (ic_launcher*, icon*) under
res/, scaled to SIZE pixels (default 76), or a plain tile if the APK has
none (its icon may be an XML drawable)."""
import io
import sys
import zipfile

from PIL import Image

apk, out = sys.argv[1], sys.argv[2]
size = int(sys.argv[3]) if len(sys.argv) > 3 else 76
z = zipfile.ZipFile(apk)
cands = [n for n in z.namelist()
         if n.startswith("res/") and n.endswith(".png")
         and n.rsplit("/", 1)[1].startswith(("ic_launcher", "icon", "app_icon"))
         and "foreground" not in n and "background" not in n]
if not cands:
    Image.new("RGBA", (size, size), (60, 130, 200, 255)).save(out)
    sys.exit(0)
best = max(cands, key=lambda n: Image.open(io.BytesIO(z.read(n))).size[0])
img = Image.open(io.BytesIO(z.read(best))).convert("RGBA")
img.resize((size, size), Image.LANCZOS).save(out)
