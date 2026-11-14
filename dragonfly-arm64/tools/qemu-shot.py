#!/usr/bin/env python3
#
# Boot the kernel in QEMU with a framebuffer in RAM (fb= boot argument),
# type commands on the serial console, and save the framebuffer as a PNG:
#
#   tools/qemu-shot.py OUT.png [--wait S] [--mem 2G] [--smp 4] \
#       [--initrd FILE] [CMD[@SECONDS] ...]
#
# Each command is sent once the previous one's time (default 3s) is up;
# the screenshot is taken at the end.  The serial log goes to OUT.log.
#
import argparse, os, socket, struct, subprocess, sys, tempfile, time, zlib

FB, W, H = 0x60000000, 1080, 1920

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("cmds", nargs="*")
ap.add_argument("--wait", type=float, default=32)
ap.add_argument("--mem", default="2G")
ap.add_argument("--smp", default="4")
ap.add_argument("--initrd")
ap.add_argument("--append", default="")
a = ap.parse_args()

here = os.path.dirname(os.path.abspath(__file__)) + "/.."
obj = os.environ.get("OBJ", here + "/work/obj")
base = os.path.splitext(os.path.abspath(a.out))[0]
mon = base + ".mon"
raw = base + ".raw"
log = open(os.path.splitext(a.out)[0] + ".log", "wb")
os.chdir(os.path.dirname(base))
raw = "fbshot.raw"
if os.path.exists(raw):
    os.unlink(raw)
qemu = subprocess.Popen([
    "qemu-system-aarch64", "-M", "virt", "-cpu", "cortex-a72",
    "-m", a.mem, "-smp", a.smp, "-display", "none", "-nic", "none",
    "-serial", "stdio", "-monitor", "unix:%s,server,nowait" % mon,
    "-kernel", os.path.abspath(obj + "/kernel.img"),
    "-initrd", os.path.abspath(a.initrd or obj + "/root.iso"),
    "-append", "vfs.root.mountfrom=cd9660:md0 fb=0x%x,%d,%d %s" %
    (FB, W, H, a.append)],
    stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT)
time.sleep(a.wait)
if qemu.poll() is not None:
    sys.exit("qemu exited early (status %s)" % qemu.returncode)
for c in a.cmds:
    cmd, _, t = c.rpartition("@") if "@" in c else (c, "", "3")
    qemu.stdin.write(cmd.encode() + b"\n")
    qemu.stdin.flush()
    time.sleep(float(t))
s = socket.socket(socket.AF_UNIX)
s.connect(mon)
s.sendall(b"pmemsave 0x%x %d %s\n" % (FB, W * H * 4, raw.encode()))
for i in range(60):
    time.sleep(0.5)
    if os.path.exists(raw) and os.path.getsize(raw) == W * H * 4:
        break
s.sendall(b"quit\n")
time.sleep(1)
qemu.kill()
data = open(raw, "rb").read()
os.unlink(raw)
# BGRX little-endian 32bpp -> RGB, halved in each direction.
rows = []
for y in range(0, H, 2):
    line = data[y * W * 4:(y + 1) * W * 4]
    px = bytearray()
    for x in range(0, W, 2):
        b, g, r = line[x * 4], line[x * 4 + 1], line[x * 4 + 2]
        px += bytes((r, g, b))
    rows.append(b"\0" + bytes(px))
def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
open(a.out, "wb").write(b"\x89PNG\r\n\x1a\n" +
    chunk(b"IHDR", struct.pack(">IIBBBBB", W // 2, H // 2, 8, 2, 0, 0, 0)) +
    chunk(b"IDAT", zlib.compress(b"".join(rows), 6)) + chunk(b"IEND", b""))
print("wrote", a.out)
