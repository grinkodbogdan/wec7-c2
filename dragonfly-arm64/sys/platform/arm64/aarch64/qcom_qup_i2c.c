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
 * MSM8996 BLSP QUP cores as I2C masters, polled.
 *
 * No BSD drives this controller; the register sequence follows Linux's
 * drivers/i2c/busses/i2c-qup.c with version 1 tags (two 16-bit tag+byte
 * entries per FIFO word), FIFO mode for short transfers and block mode
 * for long reads.  Each transfer is one message (a write, or a read of
 * up to 256 bytes); callers combine them, as RMI4 and the register
 * helpers below do.
 *
 * The Mi 5 uses two buses (device tree numbering):
 *	i2c12	BLSP2 QUP6, 0x75ba000, pins 87/88: the touch controller
 *	i2c9	BLSP2 QUP3, 0x75b7000, pins 51/52: vibrator, key backlight,
 *		speaker amplifier, USB-C controllers
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	GCC_BASE		0x00300000UL
#define	GCC_APCS_VOTE		(GCC_BASE + 0x52004)
#define	  VOTE_BLSP2_AHB	(1U << 15)
#define	GCC_BLSP2_AHB_CBCR	(GCC_BASE + 0x25004)
#define	CBCR_OFF		(1U << 31)

#define	TLMM_BASE		0x01010000UL
#define	TLMM_CFG(n)		(TLMM_BASE + 0x1000 * (n))
#define	  CFG_FUNC(f)		((f) << 2)
#define	  CFG_DRV(ma)		((((ma) / 2) - 1) << 6)

#define	QUP_CONFIG		0x000
#define	QUP_STATE		0x004
#define	  ST_RESET		0
#define	  ST_RUN		1
#define	  ST_PAUSE		3
#define	  ST_MASK		3
#define	  ST_VALID		(1U << 2)
#define	  ST_MAST_GEN		(1U << 4)
#define	QUP_IO_MODE		0x008
#define	  IO_IN_BLK		(1U << 12)
#define	  IO_REPACK		(3U << 14)
#define	QUP_SW_RESET		0x00c
#define	QUP_OPERATIONAL		0x018
#define	  OP_OUT_SVC		(1U << 8)
#define	  OP_IN_SVC		(1U << 9)
#define	  OP_MX_OUT_DONE	(1U << 10)
#define	  OP_IN_BLK_REQ		(1U << 13)
#define	QUP_ERROR_FLAGS		0x01c
#define	QUP_HW_VERSION		0x030
#define	QUP_MX_OUTPUT_CNT	0x100
#define	QUP_OUT_FIFO		0x110
#define	QUP_MX_WRITE_CNT	0x150
#define	QUP_MX_INPUT_CNT	0x200
#define	QUP_MX_READ_CNT		0x208
#define	QUP_IN_FIFO		0x218
#define	QUP_I2C_CLK_CTL		0x400
#define	QUP_I2C_STATUS		0x404
#define	  I2S_NACK		(1U << 3)
#define	  I2S_BUS_ACTIVE	(1U << 8)
#define	  I2S_ERROR_MASK	0x38000fc
#define	QUP_ERR_MASK		0x7c

#define	CFG_MINI_CORE		(2U << 8)
#define	CFG_N_VAL		15
#define	CFG_NO_INPUT		(1U << 7)
#define	TAG_START		(1U << 8)
#define	TAG_DATA		(2U << 8)
#define	TAG_STOP		(3U << 8)
#define	TAG_REC			(4U << 8)

/* 400kHz from the 19.2MHz crystal (Linux's fast-mode formula) */
#define	SRC_HZ			19200000
#define	BUS_HZ			400000
#define	FS_DIV			(((SRC_HZ / BUS_HZ) - 6) * 2 / 3)
#define	CLK_CTL			(((FS_DIV / 2) << 16) | (3 << 8) | FS_DIV)

struct qup_i2c {
	int		num;
	vm_paddr_t	base;
	vm_paddr_t	cbcr;		/* the core clock branch */
	vm_paddr_t	rcg;		/* and its root (CMD; CFG at +4) */
	int		sda, scl, func;
	int		up;		/* 1 up, -1 failed */
	int		in_fifo, out_fifo, in_blk;
	struct lock	lk;
};

static struct qup_i2c buses[] = {
	{ .num = 12, .base = 0x075ba000UL, .cbcr = GCC_BASE + 0x30008,
	  .rcg = GCC_BASE + 0x30020, .sda = 87, .scl = 88, .func = 3 },
	{ .num = 9, .base = 0x075b7000UL, .cbcr = GCC_BASE + 0x2a008,
	  .rcg = GCC_BASE + 0x2a020, .sda = 51, .scl = 52, .func = 4 },
};

static __inline uint32_t
rd(vm_paddr_t pa)
{
	return (*(volatile uint32_t *)PHYS_TO_DMAP(pa));
}

static __inline void
wr(vm_paddr_t pa, uint32_t v)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(pa) = v;
}

#define	QR(b, r)	rd((b)->base + (r))
#define	QW(b, r, v)	wr((b)->base + (r), (v))

static int
qup_poll_state(struct qup_i2c *b, uint32_t want, uint32_t mask)
{
	uint32_t st;
	int i;

	for (i = 0; i < 1000; ++i) {
		st = QR(b, QUP_STATE);
		if ((st & ST_VALID) && (st & mask) == want)
			return (0);
		DELAY(1);
	}
	return (ETIMEDOUT);
}

static int
qup_set_state(struct qup_i2c *b, uint32_t state)
{
	if (qup_poll_state(b, 0, 0))
		return (EIO);
	QW(b, QUP_STATE, state);
	return (qup_poll_state(b, state, ST_MASK));
}

int
qup_i2c_reset(struct qup_i2c *b)
{
	QW(b, QUP_SW_RESET, 1);
	if (qup_poll_state(b, ST_RESET, ST_MASK))
		return (EIO);
	QW(b, QUP_CONFIG, CFG_MINI_CORE | CFG_N_VAL);
	return (0);
}

static void
qup_bringup(struct qup_i2c *b)
{
	static const int bs[] = { 4, 16, 32, 32 };
	uint32_t io;
	int i;

	wr(GCC_APCS_VOTE, rd(GCC_APCS_VOTE) | VOTE_BLSP2_AHB);
	/* The core clock: 19.2MHz straight from the crystal. */
	wr(b->rcg + 4, (0 << 8) | 1);
	wr(b->rcg, rd(b->rcg) | 1);
	for (i = 0; i < 1000 && (rd(b->rcg) & 1); ++i)
		DELAY(1);
	wr(b->cbcr, rd(b->cbcr) | 1);
	for (i = 0; i < 1000 && (rd(b->cbcr) & CBCR_OFF); ++i)
		DELAY(1);
	for (i = 0; i < 1000 && (rd(GCC_BLSP2_AHB_CBCR) & CBCR_OFF); ++i)
		DELAY(1);
	wr(TLMM_CFG(b->sda), CFG_FUNC(b->func) | CFG_DRV(2));
	wr(TLMM_CFG(b->scl), CFG_FUNC(b->func) | CFG_DRV(2));

	if (qup_i2c_reset(b) != 0) {
		kprintf("i2c%d: QUP does not leave reset\n", b->num);
		b->up = -1;
		return;
	}
	io = QR(b, QUP_IO_MODE);
	b->in_blk = bs[(io >> 5) & 3] / 2;
	b->out_fifo = (bs[io & 3] / 2) * (2 << ((io >> 2) & 7));
	b->in_fifo = b->in_blk * (2 << ((io >> 7) & 7));
	b->up = 1;
	kprintf("i2c%d: QUP 0x%08x at 0x%jx, fifo %d/%d\n", b->num,
	    QR(b, QUP_HW_VERSION), (uintmax_t)b->base, b->in_fifo,
	    b->out_fifo);
}

/* Bus n, brought up on first use; NULL if it is not there. */
struct qup_i2c *
qup_i2c_bus(int n)
{
	struct qup_i2c *b = NULL;

	if (!msm8996_is_present())
		return (NULL);
	for (size_t i = 0; i < NELEM(buses); ++i)
		if (buses[i].num == n)
			b = &buses[i];
	if (b == NULL)
		return (NULL);
	if (b->up == 0) {
		lockinit(&b->lk, "qupi2c", 0, 0);
		qup_bringup(b);
	}
	return (b->up > 0 ? b : NULL);
}

void
qup_i2c_lock(struct qup_i2c *b)
{
	lockmgr(&b->lk, LK_EXCLUSIVE);
}

void
qup_i2c_unlock(struct qup_i2c *b)
{
	lockmgr(&b->lk, LK_RELEASE);
}

/* Errors become EIO, or ENXIO for a NACK (nobody at that address). */
static int
qup_check(struct qup_i2c *b)
{
	uint32_t bus = QR(b, QUP_I2C_STATUS) & I2S_ERROR_MASK;
	uint32_t err = QR(b, QUP_ERROR_FLAGS) & QUP_ERR_MASK;

	if (bus == 0 && err == 0)
		return (0);
	if (err)
		QW(b, QUP_ERROR_FLAGS, err);
	if (bus)
		QW(b, QUP_I2C_STATUS, bus);
	QW(b, QUP_STATE, ST_RESET);
	return ((bus & I2S_NACK) ? ENXIO : EIO);
}

/*
 * One message: a write of (len) bytes, or a read of (len) bytes (1..256)
 * into (buf), as Linux's qup_i2c_write_one / qup_i2c_read_one do.
 * The caller holds the bus lock.
 */
int
qup_i2c_xfer(struct qup_i2c *b, int addr, int rd_flag, uint8_t *buf, int len)
{
	uint32_t cfg = CFG_MINI_CORE | CFG_N_VAL, io = IO_REPACK, op, w;
	uint16_t tags[2 + 64];
	int ntags, i, pos, blk, error, t;

	if (len < 1 || len > (rd_flag ? 256 : 63))
		return (EINVAL);
	if (qup_poll_state(b, ST_MAST_GEN, ST_MAST_GEN))
		return (EIO);

	if (rd_flag) {
		tags[0] = TAG_START | (addr << 1) | 1;
		tags[1] = TAG_REC | (len & 0xff);
		ntags = 2;
	} else {
		tags[0] = TAG_START | (addr << 1);
		for (i = 0; i < len; ++i)
			tags[1 + i] = (i == len - 1 ? TAG_STOP : TAG_DATA) |
			    buf[i];
		ntags = len + 1;
	}
	if (ntags > b->out_fifo)
		return (EINVAL);
	QW(b, QUP_MX_OUTPUT_CNT, 0);
	QW(b, QUP_MX_WRITE_CNT, ntags);
	blk = rd_flag && len > b->in_fifo;
	if (!rd_flag)
		cfg |= CFG_NO_INPUT;
	else if (blk) {
		io |= IO_IN_BLK;
		QW(b, QUP_MX_READ_CNT, 0);
		QW(b, QUP_MX_INPUT_CNT, len);
	} else {
		QW(b, QUP_MX_INPUT_CNT, 0);
		QW(b, QUP_MX_READ_CNT, len);
	}
	QW(b, QUP_CONFIG, cfg);
	QW(b, QUP_IO_MODE, io);

	if ((error = qup_set_state(b, ST_RUN)) != 0)
		return (error);
	QW(b, QUP_I2C_CLK_CTL, CLK_CTL);
	if ((error = qup_set_state(b, ST_PAUSE)) != 0)
		return (error);
	for (i = 0; i < ntags; i += 2) {
		w = tags[i];
		if (i + 1 < ntags)
			w |= (uint32_t)tags[i + 1] << 16;
		QW(b, QUP_OUT_FIFO, w);
	}
	if ((error = qup_set_state(b, ST_RUN)) != 0)
		return (error);

	pos = 0;
	w = 0;
	for (t = 0; t < 20000; ++t) {		/* about 20ms */
		if ((error = qup_check(b)) != 0)
			return (error);
		op = QR(b, QUP_OPERATIONAL);
		if (!rd_flag) {
			if (op & (OP_OUT_SVC | OP_MX_OUT_DONE)) {
				QW(b, QUP_OPERATIONAL, op & (OP_OUT_SVC |
				    OP_MX_OUT_DONE));
				break;
			}
		} else if (op & OP_IN_SVC) {
			int avail;

			QW(b, QUP_OPERATIONAL, OP_IN_SVC);
			avail = blk ? b->in_blk : b->in_fifo;
			if (blk && (op & OP_IN_BLK_REQ) == 0)
				avail = 0;
			for (i = 0; avail > 0 && pos < len; ++i, --avail) {
				if ((i & 1) == 0) {
					w = QR(b, QUP_IN_FIFO);
					buf[pos++] = w & 0xff;
				} else
					buf[pos++] = (w >> 16) & 0xff;
			}
			if (pos == len)
				break;
		}
		DELAY(1);
	}
	if (t == 20000)
		return (ETIMEDOUT);
	if ((error = qup_check(b)) != 0)
		return (error);
	for (i = 0; i < 5000 && (QR(b, QUP_I2C_STATUS) & I2S_BUS_ACTIVE); ++i)
		DELAY(1);
	qup_set_state(b, ST_RESET);
	return (i == 5000 ? ETIMEDOUT : 0);
}

/* One-byte register access for simple chips (8-bit register numbers). */
int
qup_i2c_regread(struct qup_i2c *b, int addr, uint8_t reg, uint8_t *v)
{
	int error;

	qup_i2c_lock(b);
	error = qup_i2c_xfer(b, addr, 0, &reg, 1);
	if (error == 0)
		error = qup_i2c_xfer(b, addr, 1, v, 1);
	if (error)
		qup_i2c_reset(b);
	qup_i2c_unlock(b);
	return (error);
}

int
qup_i2c_regwrite(struct qup_i2c *b, int addr, uint8_t reg, uint8_t v)
{
	uint8_t buf[2] = { reg, v };
	int error;

	qup_i2c_lock(b);
	error = qup_i2c_xfer(b, addr, 0, buf, 2);
	if (error)
		qup_i2c_reset(b);
	qup_i2c_unlock(b);
	return (error);
}
