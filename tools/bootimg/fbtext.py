#!/usr/bin/env python3
"""Read a framebuffer console back as text (for tests): every console cell
is matched against the kernel's font (kernel/src/font.rs).

    fbtext.py DUMP WIDTH HEIGHT STRIDE BPP SCALE [PNG]

DUMP is the raw framebuffer memory; PNG, if given, gets a picture of it.
"""
import os
import re
import sys

dump, width, height, stride, bpp, scale = sys.argv[1], *map(int, sys.argv[2:7])
font_rs = open(os.path.join(os.path.dirname(__file__), "../../kernel/src/font.rs")).read()
fw, fh = int(re.search(r"WIDTH: usize = (\d+)", font_rs).group(1)), int(re.search(r"HEIGHT: usize = (\d+)", font_rs).group(1))
glyphs = {}
values = [int(v, 16) for v in re.findall(r"0x[0-9a-f]+", font_rs[font_rs.index("GLYPHS"):])]
for i in range(len(values) // fh):
    glyphs[tuple(values[i * fh:(i + 1) * fh])] = chr(32 + i)
data = open(dump, "rb").read()
cw, ch = fw * scale, fh * scale
cols, rows = width // cw, height // ch
x0, y0 = (width % cw) // 2, (height % ch) // 2


def lit(x, y):
    o = y * stride + x * bpp
    return data[o + 1] > 0x80  # green channel: the console draws grey on black


lines = []
for r in range(rows):
    line = ""
    for c in range(cols):
        bits = tuple(
            sum(1 << (fw - 1 - gx) for gx in range(fw) if lit(x0 + c * cw + gx * scale, y0 + r * ch + gy * scale))
            for gy in range(fh))
        line += glyphs.get(bits, "░")
    lines.append(line.rstrip())
print("\n".join(lines))
if len(sys.argv) > 7:
    from PIL import Image
    img = Image.new("RGB", (width, height))
    px = bytearray()
    for y in range(height):
        row = data[y * stride:y * stride + width * bpp]
        for x in range(width):
            b, g, r = row[x * bpp:x * bpp + 3]
            px += bytes((r, g, b))
    img.frombytes(bytes(px))
    img.save(sys.argv[7])
