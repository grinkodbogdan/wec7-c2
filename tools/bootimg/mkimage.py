#!/usr/bin/env python3
"""Package a Flare kernel for a board's boot loader (`cargo xtask image`).

    mkimage.py SPEC.json Image OUT_DIR
    mkimage.py --check boot.img SPEC.json

SPEC is the board's [boot-image] section as JSON. Supported format:
"android-v0", the Android boot image that Qualcomm's LK (and most phone
boot loaders) boot: a header page, the kernel (here a gzip-compressed Linux
arm64 Image with a flattened device tree appended, "Image.gz-dtb"), and a
ramdisk. LK chooses an appended device tree by its qcom,msm-id and
qcom,board-id, and edits /memory and /chosen before starting the kernel,
so the tree carries just those. Flare itself reads the board description
compiled into the kernel, not the tree.
"""
import gzip
import hashlib
import io
import json
import os
import struct
import zlib
import sys

# ---------------------------------------------------------------- FDT


class Fdt:
    def __init__(self):
        self.struct = bytearray()
        self.strings = bytearray()
        self.offsets = {}

    def _name(self, n):
        if n not in self.offsets:
            self.offsets[n] = len(self.strings)
            self.strings += n.encode() + b"\0"
        return self.offsets[n]

    def _pad(self):
        while len(self.struct) % 4:
            self.struct.append(0)

    def begin(self, name):
        self.struct += struct.pack(">I", 1) + name.encode() + b"\0"
        self._pad()

    def end(self):
        self.struct += struct.pack(">I", 2)

    def prop(self, name, value):
        if isinstance(value, str):
            data = value.encode() + b"\0"
        elif isinstance(value, list) and value and isinstance(value[0], str):
            data = b"".join(v.encode() + b"\0" for v in value)
        elif isinstance(value, list):
            data = b"".join(struct.pack(">I", v) for v in value)
        else:
            data = bytes(value)
        self.struct += struct.pack(">III", 3, len(data), self._name(name)) + data
        self._pad()

    def blob(self):
        self.struct += struct.pack(">I", 9)
        rsv = b"\0" * 16
        off_rsv = 40
        off_struct = off_rsv + len(rsv)
        off_strings = off_struct + len(self.struct)
        total = off_strings + len(self.strings)
        hdr = struct.pack(">10I", 0xD00DFEED, total, off_struct, off_strings, off_rsv, 17, 16, 0,
                          len(self.strings), len(self.struct))
        return hdr + rsv + bytes(self.struct) + bytes(self.strings)


def board_dtb(spec):
    t = Fdt()
    t.begin("")
    t.prop("#address-cells", [2])
    t.prop("#size-cells", [2])
    t.prop("model", spec.get("dtb-model", "Flare OS"))
    t.prop("compatible", spec["dtb-compatible"])
    if "qcom-msm-id" in spec:
        t.prop("qcom,msm-id", spec["qcom-msm-id"])
    if "qcom-board-id" in spec:
        t.prop("qcom,board-id", spec["qcom-board-id"])
    t.begin("chosen")
    t.prop("bootargs", spec.get("cmdline", ""))
    t.end()
    t.begin("memory")
    t.prop("device_type", "memory")
    # the boot loader fills in the real banks
    t.prop("reg", [0, spec["base"], 0, 0])
    t.end()
    t.end()
    return t.blob()

# ---------------------------------------------------------------- boot image


def empty_ramdisk():
    """A gzip'd newc cpio archive holding only its trailer."""
    name = b"TRAILER!!!\0"
    hdr = b"070701" + b"".join(b"%08X" % v for v in [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, len(name), 0])
    body = hdr + name
    body += b"\0" * (-len(body) % 4)
    body += b"\0" * (-len(body) % 512)
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as g:
        g.write(body)
    return out.getvalue()


def pad(b, page):
    return b + b"\0" * (-len(b) % page)


def android_v0(spec, kernel, ramdisk):
    page = spec.get("page-size", 2048)
    base = spec["base"]
    sha = hashlib.sha1()
    for blob in (kernel, ramdisk, b""):
        sha.update(blob)
        sha.update(struct.pack("<I", len(blob)))
    cmdline = spec.get("cmdline", "").encode()
    if len(cmdline) > 511:
        sys.exit("mkimage.py: command line longer than 511 bytes")
    hdr = b"ANDROID!" + struct.pack(
        "<10I", len(kernel), base + spec["kernel-offset"], len(ramdisk), base + spec["ramdisk-offset"], 0,
        base + 0x00f00000, base + spec["tags-offset"], page, 0, 0)
    hdr += b"flare".ljust(16, b"\0") + cmdline.ljust(512, b"\0") + sha.digest().ljust(32, b"\0")
    hdr += b"\0" * 1024  # extra_cmdline
    return pad(hdr, page) + pad(kernel, page) + pad(ramdisk, page)

# ---------------------------------------------------------------- check


def check(path, spec):
    """Parse a boot image back and verify what the boot loader relies on."""
    data = open(path, "rb").read()
    assert data[:8] == b"ANDROID!", "no Android boot image magic"
    ksize, kaddr, rsize, raddr, _, _, taddr, page, hver, _ = struct.unpack("<10I", data[8:48])
    assert hver == 0 and page == spec.get("page-size", 2048), "header version / page size"
    kern = data[page:page + ksize]
    assert kern[:2] == b"\x1f\x8b", "kernel is not gzip"
    # one gzip stream, then the appended device tree
    z = zlib.decompressobj(31)
    image = z.decompress(kern)
    dtb = z.unused_data
    assert image[56:60] == b"ARM\x64", "not an arm64 Image"
    text_offset, image_size = struct.unpack("<QQ", image[8:24])
    end = spec["base"] + text_offset + image_size
    assert dtb[:4] == b"\xd0\x0d\xfe\xed", "no device tree after the kernel"
    blob = dtb
    for needle in [b"qcom,msm-id", b"qcom,board-id", b"memory", b"chosen", b"device_type"]:
        if needle.startswith(b"qcom") and "qcom-msm-id" not in spec:
            continue
        assert needle in blob, "device tree lacks %s" % needle.decode()
    if "max-kernel-end" in spec:
        assert end <= spec["max-kernel-end"], "kernel ends at %#x, past %#x" % (end, spec["max-kernel-end"])
    print("mkimage.py: %s: Android boot image v0, page %d; kernel %d KiB gzip (Image %d KiB, "
          "loaded %#x..%#x), device tree %d bytes, ramdisk %d bytes: ok"
          % (os.path.basename(path), page, ksize // 1024, len(image) // 1024, spec["base"] + text_offset, end,
             len(dtb), rsize))


def main():
    if sys.argv[1] == "--check":
        check(sys.argv[2], json.load(open(sys.argv[3])))
        return
    spec = json.load(open(sys.argv[1]))
    image = open(sys.argv[2], "rb").read()
    out = sys.argv[3]
    fmt = spec.get("format")
    if fmt != "android-v0":
        sys.exit("mkimage.py: unsupported boot image format %r" % fmt)
    assert image[56:60] == b"ARM\x64", "kernel lacks the arm64 Image header"
    text_offset, image_size = struct.unpack("<QQ", image[8:24])
    end = spec["base"] + text_offset + image_size
    if "max-kernel-end" in spec and end > spec["max-kernel-end"]:
        sys.exit("mkimage.py: the kernel would occupy up to %#x, past %#x (max-kernel-end): use a smaller "
                 "root filesystem" % (end, spec["max-kernel-end"]))
    if spec.get("kernel-compression") == "gzip":
        kernel = gzip.compress(image, 9, mtime=0)
    else:
        kernel = image
    dtb = board_dtb(spec)
    os.makedirs(out, exist_ok=True)
    open(os.path.join(out, "board.dtb"), "wb").write(dtb)
    img = android_v0(spec, kernel + dtb, empty_ramdisk())
    path = os.path.join(out, "boot.img")
    open(path, "wb").write(img)
    check(path, spec)


main()
