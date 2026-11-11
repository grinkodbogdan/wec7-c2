#!/usr/bin/env python3
#
# Pack the DragonFly/arm64 kernel and ramdisk into an Android boot image
# (header version 0) that a phone's bootloader can load with
#
#	fastboot boot boot.img
#
# which runs it once from RAM without writing to the phone's storage.
#
# The kernel is stored the way Qualcomm bootloaders of the Snapdragon 820
# era expect a 64-bit kernel: gzip compressed, with the device tree
# blob(s) appended ("Image.gz-dtb").
#
# The device tree describes the phone's hardware and has to come from
# the phone.  The easy source is the boot image already on it:
#
#	tools/mkbootimg.py --stock stock-boot.img ...
#
# copies the load addresses and page size from that image's header and
# reuses every device tree blob found in it.  --dtb FILE uses a specific
# blob instead.
#
# Usage:
#	mkbootimg.py --kernel kernel.img --ramdisk root.iso \
#	    (--stock stock-boot.img | --dtb phone.dtb [--dtb ...]) \
#	    [--cmdline "..."] -o boot.img
#	mkbootimg.py --info boot.img
#
import argparse
import gzip
import hashlib
import struct
import sys

MAGIC = b"ANDROID!"
FDT_MAGIC = b"\xd0\x0d\xfe\xed"
HEADER = "<8s10I16s512s32s1024s"	# through extra_cmdline

# Defaults for MSM8996 (Snapdragon 820) boards, used without --stock.
DEFAULTS = dict(kernel_addr=0x80008000, ramdisk_addr=0x81000000,
		second_addr=0x80f00000, tags_addr=0x80000100, page_size=4096)


def parse(image):
	"""Return the header fields and payloads of a boot image."""
	if image[:8] != MAGIC:
		sys.exit("not an Android boot image (bad magic)")
	f = struct.unpack_from(HEADER, image)
	hdr = dict(kernel_size=f[1], kernel_addr=f[2], ramdisk_size=f[3],
		   ramdisk_addr=f[4], second_size=f[5], second_addr=f[6],
		   tags_addr=f[7], page_size=f[8], header_version=f[9],
		   os_version=f[10], name=f[11].rstrip(b"\0"),
		   cmdline=f[12].rstrip(b"\0") + f[14].rstrip(b"\0"))
	page = hdr["page_size"]
	if hdr["header_version"] > 2:
		sys.exit("boot image header version %d is not supported "
			 "(this tool handles 0-2)" % hdr["header_version"])

	def pages(n):
		return (n + page - 1) // page * page

	off = page
	hdr["kernel"] = image[off:off + hdr["kernel_size"]]
	off += pages(hdr["kernel_size"])
	hdr["ramdisk"] = image[off:off + hdr["ramdisk_size"]]
	return hdr


def find_dtbs(blob):
	"""Every flattened device tree in (blob), in order."""
	dtbs = []
	pos = blob.find(FDT_MAGIC)
	while pos != -1:
		size, off_struct, off_strings = struct.unpack_from(">3I", blob, pos + 4)
		version = struct.unpack_from(">I", blob, pos + 20)[0]
		if (64 <= size <= len(blob) - pos and off_struct < size and
		    off_strings < size and 16 <= version <= 17):
			dtbs.append(blob[pos:pos + size])
			pos = blob.find(FDT_MAGIC, pos + size)
		else:
			pos = blob.find(FDT_MAGIC, pos + 4)
	return dtbs


def build(args):
	geometry = dict(DEFAULTS)
	dtbs = []
	if args.stock:
		stock = parse(open(args.stock, "rb").read())
		for key in geometry:
			geometry[key] = stock[key]
		dtbs = find_dtbs(stock["kernel"])
		print("stock image: %d device tree blob(s), kernel at 0x%x, "
		      "page size %d" % (len(dtbs), stock["kernel_addr"],
					 stock["page_size"]))
		print("stock cmdline: %s" % stock["cmdline"].decode("ascii", "replace"))
	for path in args.dtb or []:
		dtbs += find_dtbs(open(path, "rb").read())
	if not dtbs:
		sys.exit("no device tree: give --stock with the phone's own "
			 "boot image, or --dtb")

	image = open(args.kernel, "rb").read()
	kernel = gzip.compress(image, 9, mtime=0)
	kernel += b"".join(dtbs)
	# These bootloaders insist on a ramdisk, even an unused one.
	ramdisk = open(args.ramdisk, "rb").read() if args.ramdisk else bytes(4096)

	# The kernel needs more memory than its file: the arm64 Image
	# header says how much (image_size, which includes the BSS).  A
	# bootloader that loads the ramdisk and device tree at fixed
	# addresses will put them inside it if it is too big.
	if image[56:60] == b"ARM\x64":
		need = struct.unpack_from("<Q", image, 16)[0] + (1 << 20)
		for what, addr in (("ramdisk", geometry["ramdisk_addr"]),
				   ("device tree", args.dtb_addr)):
			room = addr - args.load_addr
			if 0 < room < need:
				sys.exit("kernel needs %d bytes from 0x%x but the %s "
					 "is loaded at 0x%x" % (need, args.load_addr,
								 what, addr))
	# The ramdisk must also end before the device tree.
	rd_end = geometry["ramdisk_addr"] + len(ramdisk)
	if geometry["ramdisk_addr"] < args.dtb_addr < rd_end:
		sys.exit("ramdisk (%d bytes at 0x%x) runs into the device tree "
			 "at 0x%x" % (len(ramdisk), geometry["ramdisk_addr"],
				      args.dtb_addr))
	cmdline = args.cmdline.encode("ascii")
	if len(cmdline) >= 512 + 1024:
		sys.exit("command line too long")

	page = geometry["page_size"]
	# The id field: SHA-1 over each payload and its length, as AOSP's
	# mkbootimg computes it.
	sha = hashlib.sha1()
	for part in (kernel, ramdisk, b""):
		sha.update(part)
		sha.update(struct.pack("<I", len(part)))
	header = struct.pack(HEADER, MAGIC,
	    len(kernel), geometry["kernel_addr"],
	    len(ramdisk), geometry["ramdisk_addr"],
	    0, geometry["second_addr"],
	    geometry["tags_addr"], page, 0, 0,
	    b"dragonfly", cmdline[:511], sha.digest(), cmdline[511:])

	def pad(data):
		return data + b"\0" * (-len(data) % page)

	image = pad(header) + pad(kernel) + pad(ramdisk)
	open(args.output, "wb").write(image)

	# Read it back the way a bootloader would.
	check = parse(image)
	assert check["kernel"] == kernel and check["ramdisk"] == ramdisk
	assert len(find_dtbs(check["kernel"])) == len(dtbs)
	print("wrote %s: %d bytes, kernel %d (gzip) + %d dtb(s), ramdisk %d"
	      % (args.output, len(image), len(kernel) - sum(map(len, dtbs)),
		 len(dtbs), len(ramdisk)))


def info(path):
	hdr = parse(open(path, "rb").read())
	for key in ("kernel_size", "kernel_addr", "ramdisk_size",
		    "ramdisk_addr", "tags_addr", "page_size", "header_version"):
		print("%-15s 0x%x" % (key, hdr[key]))
	print("%-15s %s" % ("name", hdr["name"].decode("ascii", "replace")))
	print("%-15s %s" % ("cmdline", hdr["cmdline"].decode("ascii", "replace")))
	print("%-15s %s" % ("kernel", "gzip" if hdr["kernel"][:2] == b"\x1f\x8b"
			    else "uncompressed"))
	print("%-15s %d" % ("dtbs", len(find_dtbs(hdr["kernel"]))))


def main():
	ap = argparse.ArgumentParser(description=__doc__)
	ap.add_argument("--info", metavar="IMAGE", help="describe a boot image")
	ap.add_argument("--kernel")
	ap.add_argument("--ramdisk")
	ap.add_argument("--stock", help="the phone's existing boot image")
	ap.add_argument("--dtb", action="append",
			help="device tree blob to append (repeatable)")
	ap.add_argument("--cmdline", default="")
	ap.add_argument("--load-addr", type=lambda v: int(v, 0),
			default=0x80080000,
			help="where the bootloader really puts the kernel "
			     "(MSM8996 LK: 0x80080000 whatever the header says)")
	ap.add_argument("--dtb-addr", type=lambda v: int(v, 0),
			default=0x82000000,
			help="where the bootloader puts the device tree "
			     "(MSM8996 LK: 0x82000000)")
	ap.add_argument("-o", "--output")
	args = ap.parse_args()
	if args.info:
		info(args.info)
	elif args.kernel and args.output:
		build(args)
	else:
		ap.error("need --info IMAGE, or --kernel and -o")


if __name__ == "__main__":
	main()
