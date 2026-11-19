#!/usr/bin/env python3
"""Draw the flarehome launcher icons (original art) as PNGs.

    python3 make_icons.py [OUTDIR]

Each icon is a squircle tile with a vertical gradient, a soft top highlight
and a drop shadow, carrying a white pictogram drawn from vector shapes.
Everything is drawn at 4x and downsampled, so edges are anti-aliased. The
PNGs are committed; rerun this after changing the art (needs Pillow).
"""
import math
import os
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

SIZE = 76          # tile size on the panel
PAD = 6            # room for the shadow
SS = 4             # supersampling factor
S = SIZE * SS
FULL = (SIZE + 2 * PAD) * SS
R = 19 * SS        # corner radius
WHITE = (255, 255, 255, 255)

FONT_DIRS = ["/usr/share/fonts/truetype/dejavu", "/usr/share/fonts/dejavu",
             os.path.join(os.path.dirname(__file__), "../../../../target/arcan/sysroot/usr/share/fonts/dejavu")]


def font(name, px):
    for d in FONT_DIRS:
        p = os.path.join(d, name)
        if os.path.exists(p):
            return ImageFont.truetype(p, px)
    raise SystemExit(f"make_icons.py: font {name} not found")


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def shade(c, f):
    return tuple(max(0, min(255, int(v * f))) for v in c)


def tile(top, bottom):
    """The squircle with its gradient, highlight, rim and shadow."""
    img = Image.new("RGBA", (FULL, FULL), (0, 0, 0, 0))
    o = PAD * SS
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, S - 1, S - 1), R, fill=255)
    # shadow
    sh = Image.new("RGBA", (FULL, FULL), (0, 0, 0, 0))
    sm = Image.new("L", (FULL, FULL), 0)
    sm.paste(mask, (o, o + 3 * SS))
    sm = sm.filter(ImageFilter.GaussianBlur(4 * SS)).point(lambda v: int(v * 0.55))
    sh.putalpha(sm)
    img = Image.alpha_composite(img, sh)
    # gradient body
    body = Image.new("RGBA", (S, S))
    d = ImageDraw.Draw(body)
    for y in range(S):
        d.line((0, y, S, y), fill=lerp(top, bottom, y / S) + (255,))
    # soft highlight on the upper half
    hl = Image.new("L", (S, S), 0)
    ImageDraw.Draw(hl).ellipse((-S * 0.35, -S * 0.95, S * 1.35, S * 0.48), fill=60)
    hl = hl.filter(ImageFilter.GaussianBlur(6 * SS))
    body = Image.alpha_composite(body, Image.merge("RGBA", (*[Image.new("L", (S, S), 255)] * 3, hl)))
    body.putalpha(mask)
    layer = Image.new("RGBA", (FULL, FULL), (0, 0, 0, 0))
    layer.paste(body, (o, o), body)
    img = Image.alpha_composite(img, layer)
    # thin light rim
    rim = Image.new("RGBA", (FULL, FULL), (0, 0, 0, 0))
    ImageDraw.Draw(rim).rounded_rectangle((o, o, o + S - 1, o + S - 1), R, outline=(255, 255, 255, 46), width=SS)
    return Image.alpha_composite(img, rim)


class Pen:
    """Draw a pictogram in tile coordinates (0..76) onto a layer."""

    def __init__(self):
        self.img = Image.new("RGBA", (FULL, FULL), (0, 0, 0, 0))
        self.d = ImageDraw.Draw(self.img)

    def p(self, x, y):
        return ((PAD + x) * SS, (PAD + y) * SS)

    def box(self, x0, y0, x1, y1):
        return (*self.p(x0, y0), *self.p(x1, y1))

    def rect(self, x0, y0, x1, y1, r=0, fill=WHITE, outline=None, width=0):
        self.d.rounded_rectangle(self.box(x0, y0, x1, y1), r * SS, fill=fill, outline=outline, width=int(width * SS))

    def ellipse(self, x0, y0, x1, y1, fill=WHITE, outline=None, width=0):
        self.d.ellipse(self.box(x0, y0, x1, y1), fill=fill, outline=outline, width=int(width * SS))

    def circle(self, cx, cy, r, **kw):
        self.ellipse(cx - r, cy - r, cx + r, cy + r, **kw)

    def poly(self, pts, fill=WHITE):
        self.d.polygon([self.p(x, y) for x, y in pts], fill=fill)

    def line(self, pts, width, fill=WHITE):
        self.d.line([self.p(x, y) for x, y in pts], fill=fill, width=int(width * SS), joint="curve")
        for x, y in (pts[0], pts[-1]):
            self.circle(x, y, width / 2, fill=fill)

    def arc(self, x0, y0, x1, y1, a0, a1, width, fill=WHITE):
        self.d.arc(self.box(x0, y0, x1, y1), a0, a1, fill=fill, width=int(width * SS))

    def text(self, cx, cy, s, name, px, fill=WHITE):
        f = font(name, px * SS)
        x, y = self.p(cx, cy)
        self.d.text((x, y), s, font=f, fill=fill, anchor="mm")

    def shadowed(self):
        """The pictogram with a subtle shadow under it."""
        a = self.img.getchannel("A").filter(ImageFilter.GaussianBlur(1.5 * SS)).point(lambda v: int(v * 0.35))
        sh = Image.new("RGBA", self.img.size, (0, 0, 0, 0))
        sh.putalpha(ImageChops.offset(a, 0, SS))
        return Image.alpha_composite(sh, self.img)


def phone(p, c):
    # a desk telephone (DejaVu's U+260E outline, filled white)
    p.text(38, 40, "\u260E", "DejaVuSans.ttf", 50)


def messages(p, c):
    p.rect(13, 17, 63, 52, 12)
    p.poly([(22, 48), (20, 62), (34, 50)])
    for x in (27, 38, 49):
        p.circle(x, 34.5, 3.6, fill=shade(c, 0.9) + (255,))


def browser(p, c):
    p.circle(38, 38, 23, fill=None, outline=WHITE, width=4)
    p.ellipse(28, 15, 48, 61, fill=None, outline=WHITE, width=3)
    p.line([(38, 15), (38, 61)], 3)
    p.line([(16, 38), (60, 38)], 3)
    p.arc(15, 2, 61, 34, 35, 145, 3)
    p.arc(15, 42, 61, 74, 215, 325, 3)


def camera(p, c):
    p.rect(14, 24, 62, 56, 8)
    p.rect(28, 19, 46, 27, 3)
    p.circle(38, 40, 12, fill=shade(c, 0.55) + (255,))
    p.circle(38, 40, 8.5, fill=(40, 44, 52, 255))
    p.circle(35, 37, 2.5, fill=(255, 255, 255, 200))
    p.circle(55, 30, 2.2, fill=(255, 200, 80, 255))


def gallery(p, c):
    p.rect(14, 18, 62, 58, 6)
    p.rect(18, 22, 58, 54, 3, fill=lerp((120, 190, 255), (70, 120, 220), 0.4) + (255,))
    p.poly([(18, 54), (31, 36), (42, 50), (48, 42), (58, 54)], fill=(60, 150, 80, 255))
    p.circle(49, 30, 4.5, fill=(255, 220, 90, 255))


def music(p, c):
    p.poly([(30, 22), (54, 16), (54, 23), (30, 29)])
    p.line([(31, 24), (31, 50)], 4)
    p.line([(53, 18), (53, 44)], 4)
    for x, y in ((25.5, 51), (47.5, 45)):
        e = Image.new("RGBA", p.img.size, (0, 0, 0, 0))
        ImageDraw.Draw(e).ellipse(p.box(x - 7, y - 5, x + 7, y + 5), fill=WHITE)
        e = e.rotate(20, center=p.p(x, y), resample=Image.BICUBIC)
        p.img.alpha_composite(e)


def maps(p, c):
    folds = [(14, 22, 30, 16, 30, 54, 14, 60), (30, 16, 46, 22, 46, 60, 30, 54), (46, 22, 62, 16, 62, 54, 46, 60)]
    for i, f in enumerate(folds):
        p.poly(list(zip(f[0::2], f[1::2])), fill=(255, 255, 255, 255 if i != 1 else 215))
    p.line([(18, 46), (28, 38), (40, 44), (58, 30)], 3, fill=shade(c, 0.8) + (255,))
    # pin
    p.circle(40, 28, 7, fill=(235, 70, 60, 255))
    p.poly([(34.5, 31), (45.5, 31), (40, 41)], fill=(235, 70, 60, 255))
    p.circle(40, 28, 2.6, fill=WHITE)


def calendar(p, c):
    p.rect(15, 16, 61, 61, 8)
    p.rect(15, 16, 61, 30, 8, fill=(220, 50, 50, 255))
    p.rect(15, 24, 61, 30, 0, fill=(220, 50, 50, 255))
    p.text(38, 23, "NOV", "DejaVuSans-Bold.ttf", 8)
    p.text(38, 46, "19", "DejaVuSans-Bold.ttf", 22, fill=(40, 40, 46, 255))


def clock(p, c):
    p.circle(38, 38, 24)
    face = shade(c, 0.9) + (255,)
    for i in range(12):
        a = math.radians(i * 30)
        r0, r1 = (17, 21) if i % 3 else (15, 21)
        p.line([(38 + r0 * math.sin(a), 38 - r0 * math.cos(a)), (38 + r1 * math.sin(a), 38 - r1 * math.cos(a))],
               2.4 if i % 3 == 0 else 1.6, fill=face)
    p.line([(38, 38), (38, 24)], 3.4, fill=face)
    p.line([(38, 38), (48, 44)], 3.4, fill=face)
    p.line([(38, 38), (30, 52)], 1.4, fill=(230, 70, 60, 255))
    p.circle(38, 38, 3, fill=(230, 70, 60, 255))


def settings(p, c):
    teeth = []
    for i in range(16):
        a = math.radians(i * 22.5)
        r = 24 if i % 2 == 0 else 18
        for da in (-6, 6):
            b = a + math.radians(da)
            teeth.append((38 + r * math.cos(b), 38 + r * math.sin(b)))
    p.poly(teeth)
    p.circle(38, 38, 9, fill=shade(c, 0.85) + (255,))


def terminal(p, c):
    p.rect(13, 17, 63, 59, 6, fill=(18, 22, 26, 255), outline=(255, 255, 255, 120), width=1.2)
    p.rect(13, 17, 63, 25, 6, fill=(70, 76, 86, 255))
    p.rect(13, 22, 63, 25, 0, fill=(70, 76, 86, 255))
    for i, col in enumerate([(240, 90, 80), (240, 190, 60), (90, 200, 90)]):
        p.circle(19 + i * 5, 21, 1.6, fill=col + (255,))
    p.line([(20, 33), (27, 39), (20, 45)], 3, fill=(110, 230, 120, 255))
    p.line([(31, 46), (43, 46)], 3, fill=(110, 230, 120, 255))


def store(p, c):
    p.poly([(17, 30), (59, 30), (56, 61), (20, 61)])
    p.arc(28, 16, 48, 40, 180, 360, 3.5)
    p.text(38, 46, "★", "DejaVuSans.ttf", 18, fill=shade(c, 0.85) + (255,))


def weather(p, c):
    sun = (255, 214, 70, 255)
    for i in range(8):
        a = math.radians(i * 45)
        p.line([(30 + 13 * math.cos(a), 30 + 13 * math.sin(a)), (30 + 18 * math.cos(a), 30 + 18 * math.sin(a))], 3,
               fill=sun)
    p.circle(30, 30, 9.5, fill=sun)
    for cx, cy, r in ((36, 49, 9), (47, 44, 11), (56, 50, 7.5)):
        p.circle(cx, cy, r)
    p.rect(30, 48, 62, 57.5, 4.5)


def notes(p, c):
    p.rect(17, 14, 57, 62, 5)
    for y in (27, 35, 43, 51):
        p.line([(23, y), (51, y)], 1.6, fill=(200, 205, 214, 255))
    p.rect(17, 14, 57, 20, 5, fill=(240, 150, 40, 255))
    # pencil
    pen = Pen()
    pen.rect(34, 44, 66, 51, 1.5, fill=(255, 205, 60, 255))
    pen.rect(62, 44, 66, 51, 1.5, fill=(240, 120, 120, 255))
    pen.poly([(34, 44), (34, 51), (27, 47.5)], fill=(245, 220, 180, 255))
    pen.poly([(30, 46), (30, 49), (27, 47.5)], fill=(60, 60, 60, 255))
    rot = pen.img.rotate(35, center=p.p(46, 47), resample=Image.BICUBIC)
    p.img.alpha_composite(rot)


def contacts(p, c):
    p.circle(38, 29, 10)
    p.d.chord(p.box(18, 42, 58, 78), 180, 360, fill=WHITE)
    p.rect(18, 59, 58, 61, 0, fill=(0, 0, 0, 0))


def apk(p, c):
    # a package: an isometric box
    top = [(38, 14), (60, 25), (38, 36), (16, 25)]
    p.poly(top, fill=(255, 255, 255, 255))
    p.poly([(16, 25), (38, 36), (38, 62), (16, 51)], fill=(255, 255, 255, 215))
    p.poly([(38, 36), (60, 25), (60, 51), (38, 62)], fill=(255, 255, 255, 170))
    p.line([(27, 19.5), (49, 30.5)], 3, fill=shade(c, 0.8) + (255,))


ICONS = {
    "phone": ((92, 214, 110), (32, 150, 70), phone),
    "messages": ((90, 170, 255), (30, 100, 220), messages),
    "browser": ((255, 170, 70), (230, 100, 20), browser),
    "camera": ((112, 120, 132), (52, 56, 64), camera),
    "gallery": ((190, 120, 240), (120, 60, 190), gallery),
    "music": ((255, 100, 160), (205, 30, 100), music),
    "maps": ((70, 205, 190), (20, 140, 130), maps),
    "calendar": ((250, 250, 252), (214, 218, 226), calendar),
    "clock": ((70, 90, 160), (25, 35, 90), clock),
    "settings": ((160, 168, 180), (92, 100, 112), settings),
    "terminal": ((70, 76, 86), (24, 26, 30), terminal),
    "store": ((255, 210, 70), (240, 150, 10), store),
    "weather": ((110, 200, 255), (40, 140, 220), weather),
    "notes": ((255, 205, 90), (235, 150, 30), notes),
    "contacts": ((140, 120, 240), (80, 60, 190), contacts),
    "android": ((150, 220, 90), (80, 160, 40), apk),
}


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    for name, (top, bottom, draw) in ICONS.items():
        base = tile(top, bottom)
        pen = Pen()
        draw(pen, bottom)
        icon = Image.alpha_composite(base, pen.shadowed())
        icon = icon.resize((FULL // SS, FULL // SS), Image.LANCZOS)
        icon.save(os.path.join(out, f"{name}.png"), optimize=True)
    print(f"make_icons.py: {len(ICONS)} icons, {FULL // SS}x{FULL // SS} px")


main()
