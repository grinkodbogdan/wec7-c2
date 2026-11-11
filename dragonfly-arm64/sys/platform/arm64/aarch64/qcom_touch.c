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
 * The Mi 5's touchscreen: a Synaptics RMI4 controller (I2C address 0x20)
 * on BLSP2 QUP6 (the device tree's i2c@75ba000, "i2c12"), pins 87/88,
 * attention on TLMM GPIO 125 (active low), reset on GPIO 89.
 *
 * No BSD has drivers for either part.  The I2C side follows Linux's
 * drivers/i2c/busses/i2c-qup.c (version 1 tags, FIFO and block mode,
 * polled instead of interrupt driven); the RMI4 side follows Linux's
 * drivers/input/rmi4 (page description table, F01 device control, F12
 * register descriptors and finger objects).
 *
 * A kernel thread brings the bus up, finds the controller, and then
 * samples the finger objects at 60Hz, reporting them through an evdev
 * multitouch device with coordinates in panel pixels.  Boot argument
 * msm8996.touch=0 skips it; msm8996.touch_verbose=1 prints touches.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/sysctl.h>

#include <dev/misc/evdev/input.h>
#include <dev/misc/evdev/evdev.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

/* TLMM */
#define	TLMM_BASE		0x01010000UL
#define	TLMM_CFG(n)		(TLMM_BASE + 0x1000 * (n))
#define	TLMM_IO(n)		(TLMM_BASE + 0x1000 * (n) + 4)
#define	  CFG_PULL_UP		3
#define	  CFG_FUNC(f)		((f) << 2)
#define	  CFG_DRV(ma)		((((ma) / 2) - 1) << 6)
#define	  CFG_OE		(1U << 9)
#define	RESET_GPIO		89
#define	AVDD_GPIO		73
#define	ATTN_GPIO		125

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

static struct qup_i2c *bus;		/* i2c12 */
static int touch_verbose;

/* ---- RMI4 over I2C ---- */

static int rmi_addr = 0x20;
static int rmi_page = -1;

static int
rmi_set_page(int page)
{
	uint8_t b[2] = { 0xff, page };
	int error;

	if (page == rmi_page)
		return (0);
	error = qup_i2c_xfer(bus, rmi_addr, 0, b, 2);
	rmi_page = error ? -1 : page;
	return (error);
}

static int
rmi_read(uint16_t reg, uint8_t *buf, int len)
{
	uint8_t a = reg & 0xff;
	int error;

	qup_i2c_lock(bus);
	if ((error = rmi_set_page(reg >> 8)) == 0 &&
	    (error = qup_i2c_xfer(bus, rmi_addr, 0, &a, 1)) == 0)
		error = qup_i2c_xfer(bus, rmi_addr, 1, buf, len);
	if (error) {
		qup_i2c_reset(bus);
		rmi_page = -1;
	}
	qup_i2c_unlock(bus);
	return (error);
}

static int
rmi_write(uint16_t reg, const uint8_t *buf, int len)
{
	uint8_t b[16];
	int error;

	if (len > 15)
		return (EINVAL);
	b[0] = reg & 0xff;
	memcpy(b + 1, buf, len);
	qup_i2c_lock(bus);
	if ((error = rmi_set_page(reg >> 8)) == 0)
		error = qup_i2c_xfer(bus, rmi_addr, 0, b, len + 1);
	if (error) {
		qup_i2c_reset(bus);
		rmi_page = -1;
	}
	qup_i2c_unlock(bus);
	return (error);
}

struct rmi_fn {
	uint8_t		num;
	uint8_t		irqs;
	uint16_t	query, cmd, ctrl, data;
};

#define	MAXFN	16
static struct rmi_fn fns[MAXFN];
static int nfns, irq_bits;

static struct rmi_fn *
rmi_find(int num)
{
	for (int i = 0; i < nfns; ++i)
		if (fns[i].num == num)
			return (&fns[i]);
	return (NULL);
}

static int
rmi_scan_pdt(void)
{
	uint8_t e[6];
	int page, a, found, error;

	nfns = irq_bits = 0;
	for (page = 0; page < 4; ++page) {
		found = 0;
		for (a = 0xe9; a >= 0x05; a -= 6) {
			error = rmi_read((page << 8) | a, e, 6);
			if (error)
				return (error);
			if (e[5] == 0 || e[5] == 0xff)
				break;
			if (nfns < MAXFN) {
				struct rmi_fn *f = &fns[nfns++];

				f->num = e[5];
				f->irqs = e[4] & 7;
				f->query = (page << 8) | e[0];
				f->cmd = (page << 8) | e[1];
				f->ctrl = (page << 8) | e[2];
				f->data = (page << 8) | e[3];
				irq_bits += f->irqs;
			}
			found = 1;
		}
		if (!found)
			break;
	}
	return (nfns ? 0 : ENXIO);
}

/* A register descriptor (F12 query 1-3, 4-6, 7-9). */
#define	MAXREG	32
struct rmi_regdesc {
	int		n;
	uint8_t		reg[MAXREG];
	uint16_t	size[MAXREG];
	uint32_t	subpk[MAXREG];
};

static int
rmi_read_regdesc(uint16_t addr, struct rmi_regdesc *d)
{
	uint8_t sp, pres[35], st[128];
	int ssize, poff = 1, i, b, reg, off, error;

	memset(d, 0, sizeof(*d));
	if ((error = rmi_read(addr, &sp, 1)) != 0)
		return (error);
	if (sp == 0 || sp > sizeof(pres))
		return (EIO);
	if ((error = rmi_read(addr + 1, pres, sp)) != 0)
		return (error);
	if (pres[0] == 0) {
		poff = 3;
		ssize = pres[1] | (pres[2] << 8);
	} else
		ssize = pres[0];
	if (ssize > (int)sizeof(st))
		return (EIO);
	if ((error = rmi_read(addr + 2, st, ssize)) != 0)
		return (error);

	off = 0;
	for (i = poff, reg = 0; i < sp; ++i) {
		for (b = 0; b < 8; ++b, ++reg) {
			int size, m;

			if ((pres[i] & (1 << b)) == 0)
				continue;
			if (d->n >= MAXREG || off >= ssize)
				return (0);
			size = st[off++];
			if (size == 0) {
				size = st[off] | (st[off + 1] << 8);
				off += 2;
			}
			if (size == 0) {
				size = st[off] | (st[off + 1] << 8);
				off += 4;
			}
			d->reg[d->n] = reg;
			d->size[d->n] = size;
			m = 0;
			do {
				if (m < 28)
					d->subpk[d->n] |= (st[off] & 0x7f) << m;
				m += 7;
			} while (off < ssize && (st[off++] & 0x80));
			d->n++;
		}
	}
	return (0);
}

static int
regdesc_index(const struct rmi_regdesc *d, int reg)
{
	for (int i = 0; i < d->n; ++i)
		if (d->reg[i] == reg)
			return (i);
	return (-1);
}

/* F12 state */
static struct rmi_fn *f12;
static int max_x, max_y, nfingers, data1_off, pkt_len;
static u_int panel_w = 1080, panel_h = 1920;

static int
f12_setup(void)
{
	struct rmi_regdesc q, c, d;
	uint8_t q0, buf[16];
	int i, k, off, error;

	if ((error = rmi_read(f12->query, &q0, 1)) != 0)
		return (error);
	if ((q0 & 1) == 0) {
		kprintf("touch: F12 without register descriptors\n");
		return (ENXIO);
	}
	if ((error = rmi_read_regdesc(f12->query + 1, &q)) != 0 ||
	    (error = rmi_read_regdesc(f12->query + 4, &c)) != 0 ||
	    (error = rmi_read_regdesc(f12->query + 7, &d)) != 0) {
		kprintf("touch: F12 register descriptors: error %d\n", error);
		return (error);
	}

	/* control 8: sensor tuning, max x and y in subpacket 0 */
	k = regdesc_index(&c, 8);
	if (k >= 0 && (c.subpk[k] & 1) && c.size[k] >= 4 &&
	    c.size[k] <= (int)sizeof(buf) &&
	    rmi_read(f12->ctrl + k, buf, c.size[k]) == 0) {
		max_x = buf[0] | (buf[1] << 8);
		max_y = buf[2] | (buf[3] << 8);
	}
	if (max_x <= 0 || max_y <= 0) {
		max_x = panel_w - 1;
		max_y = panel_h - 1;
	}

	/* data 1: the finger objects, 8 bytes each; data 0 comes first */
	off = 0;
	k = regdesc_index(&d, 0);
	if (k >= 0)
		off += d.size[k];
	k = regdesc_index(&d, 1);
	if (k < 0) {
		kprintf("touch: F12 has no object data\n");
		return (ENXIO);
	}
	data1_off = off;
	nfingers = d.size[k] / 8;
	if (nfingers > 10)
		nfingers = 10;
	pkt_len = data1_off + nfingers * 8;
	if (pkt_len > 256)
		return (ENXIO);

	kprintf("touch: F12: %d query, %d control, %d data registers; "
	    "%d fingers at data+%d, range %dx%d\n", q.n, c.n, d.n,
	    nfingers, data1_off, max_x, max_y);
	if (touch_verbose)
		for (i = 0; i < d.n; ++i)
			kprintf("touch:   data%d size %d subpackets 0x%x\n",
			    d.reg[i], d.size[i], d.subpk[i]);
	return (0);
}

static int
f01_setup(void)
{
	struct rmi_fn *f01 = rmi_find(0x01);
	uint8_t q[21], st = 0, ctl, en[4];
	int i, error;

	if (f01 == NULL)
		return (ENXIO);
	memset(q, 0, sizeof(q));
	if ((error = rmi_read(f01->query, q, sizeof(q))) != 0)
		return (error);
	rmi_read(f01->data, &st, 1);
	kprintf("touch: F01 manufacturer %d, product \"%.10s\", status 0x%02x\n",
	    q[0], (char *)&q[11], st);

	/* awake, no doze, configured; every interrupt source enabled */
	if (rmi_read(f01->ctrl, &ctl, 1) == 0) {
		ctl = (ctl & ~3) | (1 << 2) | (1 << 7);
		rmi_write(f01->ctrl, &ctl, 1);
	}
	for (i = 0; i < (irq_bits + 7) / 8 && i < 4; ++i)
		en[i] = 0xff;
	if (i > 0)
		rmi_write(f01->ctrl + 1, en, i);
	return (0);
}

/* ---- evdev ---- */

static struct evdev_dev *touch_evdev;

static void
touch_register_evdev(void)
{
	struct evdev_dev *e = evdev_alloc();

	evdev_set_name(e, "Mi 5 touchscreen");
	evdev_set_phys(e, "blsp2-qup6-i2c/rmi4");
	evdev_set_id(e, BUS_I2C, 0x06cb, 0, 0);
	evdev_support_prop(e, INPUT_PROP_DIRECT);
	evdev_support_event(e, EV_SYN);
	evdev_support_event(e, EV_ABS);
	evdev_support_abs(e, ABS_MT_SLOT, 0, nfingers - 1, 0, 0, 0);
	evdev_support_abs(e, ABS_MT_TRACKING_ID, -1, 0xffff, 0, 0, 0);
	evdev_support_abs(e, ABS_MT_POSITION_X, 0, panel_w - 1, 0, 0, 0);
	evdev_support_abs(e, ABS_MT_POSITION_Y, 0, panel_h - 1, 0, 0, 0);
	evdev_support_abs(e, ABS_MT_PRESSURE, 0, 255, 0, 0, 0);
	evdev_set_flag(e, EVDEV_FLAG_MT_AUTOREL);
	if (evdev_register(e) != 0) {
		evdev_free(e);
		return;
	}
	touch_evdev = e;
}

static volatile u_int touch_frames, touch_contacts;

SYSCTL_UINT(_machdep, OID_AUTO, touch_contacts, CTLFLAG_RD,
    __DEVOLATILE(u_int *, &touch_contacts), 0, "touches seen");

static void
touch_report(const uint8_t *pkt)
{
	static uint32_t down;		/* fingers down last frame */
	union evdev_mt_slot s;
	uint32_t now = 0;
	int i, x, y;

	for (i = 0; i < nfingers; ++i) {
		const uint8_t *o = pkt + data1_off + 8 * i;

		if (o[0] == 0)
			continue;
		now |= 1U << i;
		x = (o[1] | (o[2] << 8)) * (int)(panel_w - 1) / max_x;
		y = (o[3] | (o[4] << 8)) * (int)(panel_h - 1) / max_y;
		if (x < 0) x = 0;
		if (y < 0) y = 0;
		if (x >= (int)panel_w) x = panel_w - 1;
		if (y >= (int)panel_h) y = panel_h - 1;
		if (touch_evdev != NULL) {
			memset(&s, 0, sizeof(s));
			s.id = i;
			s.x = x;
			s.y = y;
			s.p = o[5];
			evdev_mt_push_slot(touch_evdev, i, &s);
		}
		if ((down & (1U << i)) == 0) {
			++touch_contacts;
			if (touch_verbose || touch_contacts <= 3)
				kprintf("touch: finger %d down at %d,%d "
				    "(raw %d,%d type %d)\n", i, x, y,
				    o[1] | (o[2] << 8), o[3] | (o[4] << 8),
				    o[0]);
		}
	}
	if (touch_evdev != NULL && (now != 0 || down != 0))
		evdev_sync(touch_evdev);
	down = now;
}

/* ---- bring-up ---- */

static int
touch_probe(void)
{
	uint8_t b;
	int error, i;
	static const int addrs[] = { 0x20, 0x2c, 0x4a };

	if ((bus = qup_i2c_bus(12)) == NULL) {
		kprintf("touch: no i2c12\n");
		return (EIO);
	}
	/* attention: input, pulled up */
	wr(TLMM_CFG(ATTN_GPIO), CFG_FUNC(0) | CFG_DRV(16) | CFG_PULL_UP);
	kprintf("touch: gpio reset %d/%s avdd %d/%s attn %d\n",
	    (rd(TLMM_IO(RESET_GPIO)) >> 1) & 1,
	    (rd(TLMM_CFG(RESET_GPIO)) & CFG_OE) ? "out" : "in",
	    (rd(TLMM_IO(AVDD_GPIO)) >> 1) & 1,
	    (rd(TLMM_CFG(AVDD_GPIO)) & CFG_OE) ? "out" : "in",
	    rd(TLMM_IO(ATTN_GPIO)) & 1);

	for (int pass = 0; pass < 2; ++pass) {
		for (i = 0; i < (int)NELEM(addrs); ++i) {
			qup_i2c_lock(bus);
			qup_i2c_reset(bus);
			error = qup_i2c_xfer(bus, addrs[i], 1, &b, 1);
			qup_i2c_unlock(bus);
			kprintf("touch: i2c 0x%02x: %s\n", addrs[i],
			    error == 0 ? "answers" : error == ENXIO ?
			    "no answer" : "bus error");
			if (error == 0 && i == 0) {
				rmi_addr = addrs[i];
				return (0);
			}
		}
		if (pass == 0) {
			/*
			 * Nothing: power the controller (avdd enable) and
			 * take it out of reset, then try again.
			 */
			kprintf("touch: powering the controller, releasing "
			    "reset\n");
			wr(TLMM_IO(AVDD_GPIO), 2);
			wr(TLMM_CFG(AVDD_GPIO), rd(TLMM_CFG(AVDD_GPIO)) |
			    CFG_OE);
			tsleep(&touch_probe, 0, "tpwr", hz / 50 + 1);
			wr(TLMM_IO(RESET_GPIO), 0);
			wr(TLMM_CFG(RESET_GPIO), CFG_FUNC(0) | CFG_DRV(2) |
			    CFG_OE);
			tsleep(&touch_probe, 0, "trst", hz / 100 + 1);
			wr(TLMM_IO(RESET_GPIO), 2);
			tsleep(&touch_probe, 0, "trst", hz / 4 + 1);
		}
	}
	return (ENXIO);
}

static void
touch_thread(void *arg __unused)
{
	uint8_t pkt[256];
	int error, i, errs = 0;
	vm_paddr_t pa;
	u_int w, h, stride, bpp;

	if (fbcons_geometry(&pa, &w, &h, &stride, &bpp) == 0 && w && h) {
		panel_w = w;
		panel_h = h;
	}
	if (touch_probe() != 0)
		goto out;
	rmi_page = -1;
	if ((error = rmi_scan_pdt()) != 0) {
		kprintf("touch: RMI4 function table: error %d\n", error);
		goto out;
	}
	for (i = 0; i < nfns; ++i)
		kprintf("touch: F%02x irqs %d query %03x ctrl %03x data %03x\n",
		    fns[i].num, fns[i].irqs, fns[i].query, fns[i].ctrl,
		    fns[i].data);
	if (f01_setup() != 0)
		kprintf("touch: no F01\n");
	if ((f12 = rmi_find(0x12)) == NULL) {
		kprintf("touch: no F12 (2D sensor); F11 is not handled yet\n");
		goto out;
	}
	if (f12_setup() != 0)
		goto out;
	touch_register_evdev();
	kprintf("touch: ready%s\n", touch_evdev ? ", /dev/input" : "");

	for (;;) {
		tsleep(&touch_thread, 0, "touch", hz / 60 ? hz / 60 : 1);
		error = rmi_read(f12->data, pkt, pkt_len);
		if (error) {
			if (++errs <= 5)
				kprintf("touch: read error %d\n", error);
			continue;
		}
		++touch_frames;
		touch_report(pkt);
	}
out:
	kprintf("touch: not available\n");
	kthread_exit();
}

/*
 * sysctl machdep.touch_inject=X*65536+Y: a tap at panel pixel X,Y through
 * the evdev device, for testing what reads it (QEMU has no touchscreen;
 * boot with touch.fake=1).
 */
static int
sysctl_touch_inject(SYSCTL_HANDLER_ARGS)
{
	union evdev_mt_slot s;
	int v = 0, error;

	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	if (touch_evdev == NULL)
		return (ENXIO);
	memset(&s, 0, sizeof(s));
	s.x = (v >> 16) & 0xffff;
	s.y = v & 0xffff;
	s.p = 40;
	evdev_mt_push_slot(touch_evdev, 0, &s);
	evdev_sync(touch_evdev);
	evdev_sync(touch_evdev);	/* no slots this frame: released */
	return (0);
}
SYSCTL_PROC(_machdep, OID_AUTO, touch_inject, CTLTYPE_INT | CTLFLAG_WR, 0, 0,
    sysctl_touch_inject, "I", "inject a tap at X*65536+Y");

static void
msm8996_touch_init(void *dummy __unused)
{
	struct thread *td;
	int enable = 1;

	if (!msm8996_is_present()) {
		int fake = 0;
		vm_paddr_t pa;
		u_int w, h, stride, bpp;

		TUNABLE_INT_FETCH("touch.fake", &fake);
		if (fake) {
			if (fbcons_geometry(&pa, &w, &h, &stride, &bpp) == 0) {
				panel_w = w;
				panel_h = h;
			}
			nfingers = 10;
			touch_register_evdev();
		}
		return;
	}
	TUNABLE_INT_FETCH("msm8996.touch", &enable);
	if (!enable)
		return;
	TUNABLE_INT_FETCH("msm8996.touch_verbose", &touch_verbose);
	kthread_create(touch_thread, NULL, &td, "touch");
}
SYSINIT(msm8996touch, SI_SUB_CONFIGURE + 1, SI_ORDER_ANY, msm8996_touch_init,
    NULL);
