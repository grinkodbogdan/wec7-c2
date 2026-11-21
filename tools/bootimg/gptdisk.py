#!/usr/bin/env python3
"""gptdisk.py OUT NAME IMAGE - a GPT disk image with one partition, named
NAME, holding IMAGE (as a phone's eMMC holds userdata). For tests: Linux's
block layer reads the partition table and names the partition
(PARTNAME=NAME), as on the phone."""
import struct
import sys
import uuid
import zlib

SECTOR = 512
out, name, image = sys.argv[1], sys.argv[2], open(sys.argv[3], "rb").read()
first = 2048
psize = (len(image) + SECTOR - 1) // SECTOR
last = first + psize - 1
total = last + 1 + 33 + 2048  # backup entries and header at the end, and slack
LINUX_DATA = uuid.UUID("0fc63daf-8483-4772-8e79-3d69d8477de4")

entry = struct.pack("<16s16sQQQ72s", LINUX_DATA.bytes_le, uuid.uuid4().bytes_le, first, last, 0,
                    name.encode("utf-16-le").ljust(72, b"\0"))
entries = entry.ljust(128 * 128, b"\0")
ecrc = zlib.crc32(entries) & 0xffffffff
disk_guid = uuid.uuid4().bytes_le


def header(my, alt, entries_lba):
    h = struct.pack("<8sIIIIQQQQ16sQIII", b"EFI PART", 0x10000, 92, 0, 0, my, alt, 2048, total - 2048 - 34,
                    disk_guid, entries_lba, 128, 128, ecrc)
    crc = zlib.crc32(h) & 0xffffffff
    return (h[:16] + struct.pack("<I", crc) + h[20:]).ljust(SECTOR, b"\0")


mbr = bytearray(SECTOR)
mbr[446:462] = struct.pack("<BBBBBBBBII", 0, 0, 2, 0, 0xEE, 0xFF, 0xFF, 0xFF, 1, min(total - 1, 0xFFFFFFFF))
mbr[510:512] = b"\x55\xaa"
with open(out, "wb") as f:
    f.truncate(total * SECTOR)
    f.seek(0)
    f.write(mbr)
    f.write(header(1, total - 1, 2))
    f.write(entries)
    f.seek(first * SECTOR)
    f.write(image)
    f.seek((total - 33) * SECTOR)
    f.write(entries)
    f.write(header(total - 1, 1, total - 33))
