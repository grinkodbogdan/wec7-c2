/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 * 1. Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in
 *    the documentation and/or other materials provided with the
 *    distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
 * ANY EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.  IN NO EVENT SHALL THE
 * AUTHOR OR CONTRIBUTORS BE LIABLE FOR ANY DAMAGES ARISING IN ANY WAY OUT
 * OF THE USE OF THIS SOFTWARE.
 */

/*
 * Read-only driver for the Snapdragon 820's UFS storage, using the
 * controller as LK leaves it.
 *
 * The full path to the flash is FreeBSD's ufshci(4) (the UFS host
 * controller standard) plus Qualcomm's clock, regulator and PHY bring-up,
 * which only Linux has (ufs-qcom, phy-qcom-qmp-ufs).  But LK reads the
 * flash itself before it starts us and leaves the controller enabled
 * with the link up, so standard UTP transfer requests work as they are.
 * This driver takes the transfer request list over and serves each LUN
 * as a disk, /dev/ufsN, whose GPT the kernel's slice code reads.
 *
 * Writes are refused (EROFS) until the driver has been trusted with the
 * phone's data.  One request at a time, polled, from a worker thread;
 * transfers go through a 128KB bounce buffer.  Boot argument
 * msm8996.ufs=0 skips it.
 *
 * Register layout and descriptor formats: JEDEC JESD223 (UFSHCI) and
 * JESD220 (UFS), as FreeBSD's ufshci_reg.h encodes them.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/malloc.h>
#include <sys/buf.h>
#include <sys/buf2.h>
#include <sys/conf.h>
#include <sys/disk.h>
#include <sys/devicestat.h>
#include <sys/fcntl.h>
#include <sys/kthread.h>
#include <sys/lock.h>
#include <sys/spinlock.h>
#include <sys/spinlock2.h>

#include <vm/vm.h>
#include <vm/pmap.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	UFS_BASE	0x624000UL		/* ufshc@624000 */

/* UFSHCI registers */
#define	UFS_CAP		0x00
#define	  CAP_NUTRS(c)	(((c) & 0x1f) + 1)
#define	  CAP_64AS	(1U << 24)
#define	UFS_VER		0x08
#define	UFS_IS		0x20
#define	UFS_IE		0x24
#define	UFS_HCS		0x30
#define	  HCS_DP	(1U << 0)	/* device present */
#define	  HCS_UTRLRDY	(1U << 1)
#define	  HCS_UCRDY	(1U << 3)
#define	UFS_HCE		0x34
#define	UFS_UTRLBA	0x50
#define	UFS_UTRLBAU	0x54
#define	UFS_UTRLDBR	0x58
#define	UFS_UTRLCLR	0x5c
#define	UFS_UTRLRSR	0x60

/* UTP transfer request descriptor (32 bytes) */
struct utrd {
	uint32_t	dw0;		/* CT, DD, I */
	uint32_t	dw1;
	uint32_t	dw2;		/* OCS in bits 7:0 */
	uint32_t	dw3;
	uint32_t	ucdba;		/* command descriptor, 128-byte aligned */
	uint32_t	ucdbau;
	uint32_t	resp;		/* offset 31:16, length 15:0, dwords */
	uint32_t	prdt;		/* offset 31:16, length 15:0 (entries) */
};
#define	UTRD_CT_UFS	(1U << 28)
#define	UTRD_DD_READ	(2U << 25)	/* device to host */
#define	UTRD_DD_NONE	(0U << 25)
#define	OCS_INVALID	0x0f

/* UTP command descriptor: command UPIU, response UPIU, PRDT */
#define	UCD_CMD		0		/* command UPIU, 512 bytes room */
#define	UCD_RESP	512		/* response UPIU, 512 bytes room */
#define	UCD_PRDT	1024
#define	UCD_SIZE	(UCD_PRDT + 16 * 16)

struct prdt_entry {
	uint32_t	addr;
	uint32_t	addru;
	uint32_t	reserved;
	uint32_t	count;		/* byte count - 1, bits 17:0 */
};

#define	DATA_SIZE	(128 * 1024)

struct ufs_probe {
	vm_paddr_t	base;
	struct utrd	*utrl;		/* 1KB aligned list */
	uint8_t		*ucd;
	uint8_t		*data;
	vm_paddr_t	utrl_pa, ucd_pa, data_pa;
	uint8_t		tag;
};

static struct ufs_probe ufs_softc;

#define	UFS_MAXLUN	8
struct ufs_lun {
	int		lun;
	uint32_t	bsize;
	uint64_t	blocks;
	struct disk	disk;
	cdev_t		dev;
	struct devstat	stats;
};
static struct ufs_lun *ufs_luns[UFS_MAXLUN];

static struct spinlock	ufs_spin = SPINLOCK_INITIALIZER(ufs_spin, "ufsq");
static struct bio_queue_head ufs_queue;
static struct thread	*ufs_td;

static __inline uint32_t
ufs_rd(struct ufs_probe *u, u_int reg)
{
	return (*(volatile uint32_t *)PHYS_TO_DMAP(u->base + reg));
}

static __inline void
ufs_wr(struct ufs_probe *u, u_int reg, uint32_t v)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(u->base + reg) = v;
}

/* The controller is not cache coherent here: clean, and invalidate. */
static void
dcache_wbinv(void *va, size_t len)
{
	vm_offset_t p = (vm_offset_t)va & ~63UL;
	vm_offset_t e = (vm_offset_t)va + len;

	for (; p < e; p += 64)
		__asm __volatile("dc civac, %0" : : "r" (p) : "memory");
	__asm __volatile("dsb sy" : : : "memory");
}

static int
wait_bits(struct ufs_probe *u, u_int reg, uint32_t mask, uint32_t want,
    int ms)
{
	while (ms-- > 0) {
		if ((ufs_rd(u, reg) & mask) == want)
			return (0);
		DELAY(1000);
	}
	return (ETIMEDOUT);
}

/*
 * Issue one SCSI command to (lun) through slot 0; (len) bytes of data
 * come back into u->data.  Returns 0, or prints why not and returns an
 * error.  On SCSI CHECK CONDITION, *sense gets the sense key.
 */
static int
ufs_scsi(struct ufs_probe *u, int lun, const uint8_t *cdb, int cdblen,
    size_t len, int *sense)
{
	struct utrd *d = &u->utrl[0];
	struct prdt_entry *pe;
	uint8_t *cmd = u->ucd + UCD_CMD;
	uint8_t *resp = u->ucd + UCD_RESP;
	uint32_t ocs;

	bzero(u->ucd, UCD_SIZE);
	bzero(u->data, DATA_SIZE);
	bzero(d, sizeof(*d));

	/* Command UPIU */
	cmd[0] = 0x01;				/* COMMAND */
	cmd[1] = len ? 0x40 : 0x00;		/* flags: read */
	cmd[2] = lun;
	cmd[3] = ++u->tag;
	cmd[4] = 0;				/* SCSI command set */
	cmd[12] = len >> 24;			/* expected transfer length */
	cmd[13] = len >> 16;
	cmd[14] = len >> 8;
	cmd[15] = len;
	bcopy(cdb, cmd + 16, cdblen);

	pe = (struct prdt_entry *)(u->ucd + UCD_PRDT);
	if (len) {
		pe->addr = (uint32_t)u->data_pa;
		pe->addru = (uint32_t)(u->data_pa >> 32);
		pe->count = len - 1;
	}

	d->dw0 = UTRD_CT_UFS | (len ? UTRD_DD_READ : UTRD_DD_NONE);
	d->dw2 = OCS_INVALID;
	d->ucdba = (uint32_t)u->ucd_pa;
	d->ucdbau = (uint32_t)(u->ucd_pa >> 32);
	d->resp = ((UCD_RESP / 4) << 16) | (512 / 4);
	d->prdt = ((UCD_PRDT / 4) << 16) | (len ? 1 : 0);

	dcache_wbinv(u->ucd, UCD_SIZE);
	dcache_wbinv(u->data, DATA_SIZE);
	dcache_wbinv(d, sizeof(*d));

	ufs_wr(u, UFS_IS, ufs_rd(u, UFS_IS));	/* clear stale status */
	ufs_wr(u, UFS_UTRLDBR, 1);
	if (wait_bits(u, UFS_UTRLDBR, 1, 0, 2000) != 0) {
		kprintf("ufs: lun %d cmd 0x%02x: no completion (IS 0x%x); "
		    "clearing\n", lun, cdb[0], ufs_rd(u, UFS_IS));
		ufs_wr(u, UFS_UTRLCLR, ~1U);
		return (ETIMEDOUT);
	}
	dcache_wbinv(d, sizeof(*d));
	dcache_wbinv(u->ucd, UCD_SIZE);
	dcache_wbinv(u->data, DATA_SIZE);

	ocs = d->dw2 & 0xff;
	if (ocs != 0) {
		kprintf("ufs: lun %d cmd 0x%02x: OCS 0x%x\n", lun, cdb[0], ocs);
		return (EIO);
	}
	if (resp[0] != 0x21 || resp[6] != 0 || resp[7] != 0) {
		/* Sense data follows the 32-byte header (2-byte length). */
		if (sense != NULL)
			*sense = resp[32 + 2 + 2] & 0x0f;
		if (sense == NULL)	/* callers with sense report it */
			kprintf("ufs: lun %d cmd 0x%02x: response 0x%x "
			    "status 0x%x sense key %d\n", lun, cdb[0],
			    resp[6], resp[7], resp[32 + 2 + 2] & 0x0f);
		return (EIO);
	}
	return (0);
}

static uint32_t
be32(const uint8_t *p)
{
	return ((uint32_t)p[0] << 24 | p[1] << 16 | p[2] << 8 | p[3]);
}

static uint64_t
le64(const uint8_t *p)
{
	uint64_t v = 0;
	int i;

	for (i = 7; i >= 0; --i)
		v = v << 8 | p[i];
	return (v);
}

/* READ(10): (count) blocks from (lba). */
static int
ufs_read(struct ufs_probe *u, int lun, uint32_t lba, int count,
    uint32_t bsize)
{
	uint8_t cdb[10] = { 0x28, 0, lba >> 24, lba >> 16, lba >> 8, lba,
	    0, count >> 8, count, 0 };

	return (ufs_scsi(u, lun, cdb, sizeof(cdb), (size_t)count * bsize,
	    NULL));
}

/* TEST UNIT READY, riding out the unit attention after reset. */
static int
ufs_ready(struct ufs_probe *u, int lun)
{
	static const uint8_t tur[6] = { 0x00 };
	int i, sense;

	for (i = 0; i < 4; ++i) {
		sense = 0;
		if (ufs_scsi(u, lun, tur, sizeof(tur), 0, &sense) == 0)
			return (0);
		if (sense != 6)
			return (EIO);
	}
	return (EIO);
}

static void
ufs_gpt(struct ufs_probe *u, uint32_t bsize)
{
	uint8_t hdr[92];
	uint64_t entlba;
	uint32_t nent, esize, per, lba, i, shown = 0;
	int c, j;

	if (ufs_read(u, 0, 1, 1, bsize) != 0)
		return;
	if (bcmp(u->data, "EFI PART", 8) != 0) {
		kprintf("ufs: lun 0: no GPT at LBA 1\n");
		return;
	}
	bcopy(u->data, hdr, sizeof(hdr));
	entlba = le64(hdr + 72);
	nent = hdr[80] | hdr[81] << 8 | hdr[82] << 16 | (uint32_t)hdr[83] << 24;
	esize = hdr[84] | hdr[85] << 8;
	if (esize < 128 || esize > bsize)
		return;
	per = bsize / esize;
	kprintf("ufs: lun 0: GPT, %u entries at LBA %ju:\n", nent,
	    (uintmax_t)entlba);
	for (lba = 0; lba * per < nent && lba < 8; ++lba) {
		if (ufs_read(u, 0, (uint32_t)entlba + lba, 1, bsize) != 0)
			return;
		for (i = 0; i < per; ++i) {
			const uint8_t *e = u->data + i * esize;
			uint64_t first = le64(e + 32), last = le64(e + 40);
			char name[37];

			if (le64(e) == 0 && le64(e + 8) == 0)
				continue;	/* unused entry */
			for (j = 0; j < 36; ++j) {
				c = e[56 + 2 * j];	/* UTF-16LE, ASCII part */
				name[j] = (c >= 0x20 && c < 0x7f) ? c : 0;
				if (c == 0)
					break;
			}
			name[36] = 0;
			kprintf("%s%-12s %6juM", shown % 3 ? "  " : "\n  ",
			    name, (uintmax_t)((last - first + 1) * bsize >> 20));
			++shown;
		}
	}
	kprintf("\nufs: lun 0: %u partitions\n", shown);
}

static d_open_t	ufs_open;
static d_close_t	ufs_close;
static d_strategy_t	ufs_strategy;

static struct dev_ops ufs_ops = {
	{ "ufs", 0, D_DISK | D_MPSAFE },
	.d_open =	ufs_open,
	.d_close =	ufs_close,
	.d_read =	physread,
	.d_write =	physwrite,
	.d_strategy =	ufs_strategy,
};

static int
ufs_open(struct dev_open_args *ap)
{
	if (ap->a_oflags & FWRITE)
		return (EROFS);
	return (0);
}

static int
ufs_close(struct dev_close_args *ap __unused)
{
	return (0);
}

static int
ufs_strategy(struct dev_strategy_args *ap)
{
	struct bio *bio = ap->a_bio;
	struct buf *bp = bio->bio_buf;

	bio->bio_driver_info = ap->a_head.a_dev->si_drv1;
	if (bp->b_cmd != BUF_CMD_READ) {
		bp->b_error = (bp->b_cmd == BUF_CMD_WRITE) ? EROFS : EOPNOTSUPP;
		bp->b_flags |= B_ERROR;
		bp->b_resid = bp->b_bcount;
		biodone(bio);
		return (0);
	}
	spin_lock(&ufs_spin);
	bioq_insert_tail(&ufs_queue, bio);
	spin_unlock(&ufs_spin);
	wakeup(&ufs_queue);
	return (0);
}

/* Serve one read, in bounce-buffer sized pieces. */
static void
ufs_do_read(struct ufs_lun *l, struct bio *bio)
{
	struct ufs_probe *u = &ufs_softc;
	struct buf *bp = bio->bio_buf;
	off_t off = bio->bio_offset;
	size_t done = 0, n;
	int error = 0;

	if (off % l->bsize || bp->b_bcount % l->bsize ||
	    off / l->bsize + bp->b_bcount / l->bsize > l->blocks) {
		error = EINVAL;
	}
	while (error == 0 && done < (size_t)bp->b_bcount) {
		n = MIN((size_t)bp->b_bcount - done, DATA_SIZE);
		error = ufs_read(u, l->lun, (uint32_t)((off + done) / l->bsize),
		    (int)(n / l->bsize), l->bsize);
		if (error == 0)
			bcopy(u->data, (char *)bp->b_data + done, n);
		done += n;
	}
	if (error) {
		bp->b_error = error;
		bp->b_flags |= B_ERROR;
		bp->b_resid = bp->b_bcount;
	} else {
		bp->b_resid = 0;
	}
}

static void
ufs_thread(void *arg __unused)
{
	struct bio *bio;
	struct ufs_lun *l;

	for (;;) {
		spin_lock(&ufs_spin);
		bio = bioq_takefirst(&ufs_queue);
		if (bio == NULL) {
			ssleep(&ufs_queue, &ufs_spin, 0, "ufsidle", 0);
			spin_unlock(&ufs_spin);
			continue;
		}
		spin_unlock(&ufs_spin);
		l = bio->bio_driver_info;
		devstat_start_transaction(&l->stats);
		ufs_do_read(l, bio);
		devstat_end_transaction_buf(&l->stats, bio->bio_buf);
		biodone(bio);
	}
}

static void
ufs_attach_lun(int lun, uint32_t bsize, uint64_t blocks, const char *vendor,
    const char *product)
{
	struct disk_info info;
	struct ufs_lun *l;

	l = kmalloc(sizeof(*l), M_DEVBUF, M_WAITOK | M_ZERO);
	l->lun = lun;
	l->bsize = bsize;
	l->blocks = blocks;
	devstat_add_entry(&l->stats, "ufs", lun, bsize,
	    DEVSTAT_NO_ORDERED_TAGS,
	    DEVSTAT_TYPE_DIRECT | DEVSTAT_TYPE_IF_OTHER,
	    DEVSTAT_PRIORITY_DISK);
	l->dev = disk_create(lun, &l->disk, &ufs_ops);
	l->dev->si_drv1 = l;
	l->dev->si_iosize_max = DATA_SIZE;
	disk_setdisktype(&l->disk, "UFS");

	bzero(&info, sizeof(info));
	info.d_media_blksize = bsize;
	info.d_media_blocks = blocks;
	info.d_secpertrack = 1024;
	info.d_nheads = 1;
	info.d_secpercyl = info.d_secpertrack * info.d_nheads;
	info.d_ncylinders = (u_int)(blocks / info.d_secpercyl);
	disk_setdiskinfo(&l->disk, &info);
	ufs_luns[lun] = l;
	kprintf("ufs%d: %s %s, %ju MB, read-only\n", lun, vendor, product,
	    (uintmax_t)(blocks * bsize >> 20));
}

static void
msm8996_ufs_probe(void *dummy __unused)
{
	static const uint8_t inquiry[6] = { 0x12, 0, 0, 0, 36, 0 };
	static const uint8_t readcap[10] = { 0x25 };
	struct ufs_probe *u = &ufs_softc;
	uint32_t cap, hcs, rsr, bsize;
	uint64_t blocks;
	char vendor[9], product[17];
	int enable = 1, lun, i, nluns = 0;

	if (!msm8996_is_present())
		return;
	TUNABLE_INT_FETCH("msm8996.ufs", &enable);
	if (!enable)
		return;

	bzero(u, sizeof(*u));
	u->base = UFS_BASE;
	cap = ufs_rd(u, UFS_CAP);
	hcs = ufs_rd(u, UFS_HCS);
	rsr = ufs_rd(u, UFS_UTRLRSR);
	if (bootverbose) {
		kprintf("ufs: controller version 0x%x, cap 0x%x; enable %d, "
		    "status 0x%x, run-stop %d\n", ufs_rd(u, UFS_VER), cap,
		    ufs_rd(u, UFS_HCE), hcs, rsr);
	}
	if ((ufs_rd(u, UFS_HCE) & 1) == 0 || (hcs & HCS_DP) == 0 ||
	    (hcs & HCS_UTRLRDY) == 0) {
		kprintf("ufs: the bootloader did not leave the controller "
		    "running; a driver has to bring it up\n");
		return;
	}

	/* Our own request list, command descriptor and data buffer. */
	u->utrl = contigmalloc(PAGE_SIZE, M_DEVBUF, M_WAITOK | M_ZERO,
	    0, ~(vm_paddr_t)0, PAGE_SIZE, 0);
	u->ucd = contigmalloc(PAGE_SIZE * 2, M_DEVBUF, M_WAITOK | M_ZERO,
	    0, ~(vm_paddr_t)0, PAGE_SIZE, 0);
	u->data = contigmalloc(DATA_SIZE, M_DEVBUF, M_WAITOK | M_ZERO,
	    0, ~(vm_paddr_t)0, PAGE_SIZE, 0);
	if (u->utrl == NULL || u->ucd == NULL || u->data == NULL) {
		kprintf("ufs: no DMA memory\n");
		return;
	}
	u->utrl_pa = vtophys(u->utrl);
	u->ucd_pa = vtophys(u->ucd);
	u->data_pa = vtophys(u->data);

	/*
	 * Take the list over: interrupts off (we poll), stop the list,
	 * point it at ours, start it again.
	 */
	ufs_wr(u, UFS_IE, 0);
	if (rsr & 1) {
		ufs_wr(u, UFS_UTRLRSR, 0);
		if (wait_bits(u, UFS_UTRLRSR, 1, 0, 100) != 0) {
			kprintf("ufs: transfer list will not stop; "
			    "leaving it alone\n");
			return;
		}
	}
	dcache_wbinv(u->utrl, PAGE_SIZE);
	ufs_wr(u, UFS_UTRLBA, (uint32_t)u->utrl_pa);
	ufs_wr(u, UFS_UTRLBAU, (uint32_t)(u->utrl_pa >> 32));
	ufs_wr(u, UFS_UTRLRSR, 1);
	cpu_mfence();

	bioq_init(&ufs_queue);
	for (lun = 0; lun < UFS_MAXLUN; ++lun) {
		if (ufs_ready(u, lun) != 0)
			continue;
		if (ufs_scsi(u, lun, inquiry, sizeof(inquiry), 36, NULL) != 0)
			continue;
		bcopy(u->data + 8, vendor, 8);
		bcopy(u->data + 16, product, 16);
		vendor[8] = product[16] = 0;
		for (i = 7; i >= 0 && vendor[i] == ' '; --i)
			vendor[i] = 0;
		for (i = 15; i >= 0 && product[i] == ' '; --i)
			product[i] = 0;
		if (ufs_scsi(u, lun, readcap, sizeof(readcap), 8, NULL) != 0)
			continue;
		blocks = (uint64_t)be32(u->data) + 1;
		bsize = be32(u->data + 4);
		if (bsize < 512 || bsize > DATA_SIZE)
			continue;
		if (bootverbose && lun == 0)
			ufs_gpt(u, bsize);
		ufs_attach_lun(lun, bsize, blocks, vendor, product);
		++nluns;
	}
	if (nluns > 0)
		kthread_create(ufs_thread, NULL, &ufs_td, "ufs");
}
SYSINIT(msm8996ufs, SI_SUB_CONFIGURE, SI_ORDER_ANY, msm8996_ufs_probe, NULL);
