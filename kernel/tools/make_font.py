#!/usr/bin/env python3
"""Render the kernel console font: printable ASCII from DejaVu Sans Mono
(Bitstream Vera license, which permits this use) into 12x24 one-bit cells,
written as kernel/src/font.rs.

    make_font.py /path/to/DejaVuSansMono.ttf
"""
import sys
from PIL import Image, ImageDraw, ImageFont

W, H, SIZE = 12, 24, 20
font = ImageFont.truetype(sys.argv[1], SIZE)
# place the ink of "|Mgj_" (cap height to descender) in the middle of the cell
top, bottom = font.getbbox("|Mgj_\u00c9")[1::2]
yoff = (H - (bottom - top)) // 2 - top
rows = []
for c in range(32, 127):
    img = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(img)
    d.text((0, yoff), chr(c), font=font, fill=255)
    px = img.load()
    glyph = []
    for y in range(H):
        bits = 0
        for x in range(W):
            if px[x, y] >= 110:
                bits |= 1 << (W - 1 - x)
        glyph.append(bits)
    rows.append(glyph)
with open(sys.argv[2] if len(sys.argv) > 2 else "src/font.rs", "w") as f:
    f.write("//! Console font: printable ASCII, 12x24 one-bit cells, rendered from\n"
            "//! DejaVu Sans Mono by tools/make_font.py. Row bit 11 is the leftmost pixel.\n\n")
    f.write(f"pub const WIDTH: usize = {W};\npub const HEIGHT: usize = {H};\n\n")
    f.write("#[rustfmt::skip]\npub static GLYPHS: [[u16; %d]; 95] = [\n" % H)
    for i, g in enumerate(rows):
        f.write("    [" + ", ".join("0x%03x" % b for b in g) + "], // %r\n" % chr(32 + i))
    f.write("];\n")
