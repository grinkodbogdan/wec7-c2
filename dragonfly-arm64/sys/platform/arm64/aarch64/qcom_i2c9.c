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
 * The Mi 5's small parts on I2C bus 9 (BLSP2 QUP3):
 *
 *	0x5a	TI DRV2604 haptic driver (the vibrator, an LRA), enable on
 *		TLMM GPIO 93		sysctl hw.mi5.vibrate=MS
 *	0x30	TI LP5562 LED driver (the capacitive keys' backlight),
 *		enable on PM8994 GPIO 7	sysctl hw.mi5.keylight=0..255
 *
 * Sequences after Linux's drivers/input/misc/drv260x.c (LRA mode:
 * auto-calibration, then real-time playback) and drivers/leds/
 * leds-lp5562.c with leds-lp55xx-common.c (reset, enable, direct PWM
 * control), with the Mi 5's settings from its upstream device tree
 * (msm8996-xiaomi-gemini.dts).  The bus also carries the speaker
 * amplifier (0x34) and two USB-C controllers (0x47, 0x67), which
 * the probe only reports.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/sysctl.h>
#include <sys/callout.h>
#include <sys/taskqueue.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	TLMM_BASE	0x01010000UL
#define	TLMM_CFG(n)	(TLMM_BASE + 0x1000 * (n))
#define	TLMM_IO(n)	(TLMM_BASE + 0x1000 * (n) + 4)
#define	  CFG_DRV(ma)	((((ma) / 2) - 1) << 6)
#define	  CFG_OE	(1U << 9)
#define	VIB_EN_GPIO	93

/* DRV2604 */
#define	DRV_ADDR	0x5a
#define	DRV_STATUS	0x00
#define	DRV_MODE	0x01
#define	  MODE_RTP		0x05
#define	  MODE_AUTO_CAL		0x07
#define	  MODE_STANDBY		0x40
#define	DRV_RTP_IN	0x02
#define	DRV_LIB_SEL	0x03
#define	  LIB_LRA		0x06
#define	DRV_GO		0x0c
#define	DRV_A2V_CTRL	0x11
#define	DRV_A2V_MIN_IN	0x12
#define	DRV_A2V_MAX_IN	0x13
#define	DRV_A2V_MIN_OUT	0x14
#define	DRV_A2V_MAX_OUT	0x15
#define	DRV_RATED_VOLT	0x16
#define	DRV_OD_CLAMP	0x17
#define	DRV_CAL_COMP	0x18
#define	DRV_CAL_BEMF	0x19
#define	DRV_FEEDBACK	0x1a
#define	DRV_CTRL1	0x1b
#define	DRV_CTRL2	0x1c
#define	DRV_CTRL3	0x1d
#define	DRV_CTRL4	0x1e

/* LP5562 */
#define	LP_ADDR		0x30
#define	LP_ENABLE	0x00
#define	  LP_MASTER_EN		0x40
#define	  LP_LOG_PWM		0x80
#define	LP_OP_MODE	0x01
#define	  LP_DIRECT		0x3f
#define	LP_B_PWM	0x02
#define	LP_G_PWM	0x03
#define	LP_R_PWM	0x04
#define	LP_B_CUR	0x05
#define	LP_G_CUR	0x06
#define	LP_R_CUR	0x07
#define	LP_CONFIG	0x08
#define	  LP_CFG_DEFAULT	0x61	/* high-frequency PWM, power save, internal clock */
#define	LP_RESET	0x0d
#define	LP_W_PWM	0x0e
#define	LP_W_CUR	0x0f
#define	LP_ENG_SEL	0x70
#define	LP_CURRENT	0x32		/* 5mA, from the device tree */

/* PM8994 GPIO 7 */
#define	PMGPIO7		0xc600
#define	  PMGPIO_MODE_CTL	0x40
#define	  PMGPIO_DIG_OUT	0x45
#define	  PMGPIO_EN_CTL		0x46

static char i2c9_status[2048] = "not started";
static size_t i2c9_status_len;
SYSCTL_DECL(_hw_mi5);
SYSCTL_STRING(_hw_mi5, OID_AUTO, i2c9, CTLFLAG_RD, i2c9_status, 0,
    "vibrator and key light bring-up progress");

/* A line of progress, kept for sysctl hw.mi5.i2c9 and printed. */
static void
st9(const char *fmt, ...)
{
	char line[256];
	__va_list ap;
	size_t n;

	__va_start(ap, fmt);
	kvsnprintf(line, sizeof(line), fmt, ap);
	__va_end(ap);
	n = strlen(line);
	if (n > 0 && line[n - 1] == '\n')
		line[--n] = 0;
	if (i2c9_status_len + n + 2 < sizeof(i2c9_status)) {
		i2c9_status[i2c9_status_len++] = '\n';
		memcpy(i2c9_status + i2c9_status_len, line, n + 1);
		i2c9_status_len += n;
	}
	kprintf("%s\n", line);
}

static struct qup_i2c *bus;
static int vib_ok, lp_ok;
static int keylight;
static struct callout vib_stop_co;
static struct task vib_stop_task;

static __inline void
wr(vm_paddr_t pa, uint32_t v)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(pa) = v;
}

static void
vib_enable(int on)
{
	wr(TLMM_IO(VIB_EN_GPIO), on ? 2 : 0);
	wr(TLMM_CFG(VIB_EN_GPIO), CFG_DRV(8) | CFG_OE);
	DELAY(250);		/* the data sheet's wait after enable */
}

static int
dw(uint8_t reg, uint8_t v)
{
	return (qup_i2c_regwrite(bus, DRV_ADDR, reg, v));
}

/*
 * The Mi 5's own driver (Xiaomi's drv2604.c) does not auto-calibrate: it
 * sets the actuator's voltages and drive time and writes fixed
 * calibration results.  Linux's generic calibration fails on this LRA
 * (status 0x88), so this follows Xiaomi: LRA at 240Hz, rated 0x30,
 * overdrive 0x40, closed loop, signed real-time playback.
 */
static int
vib_init(void)
{
	uint8_t st = 0, fb = 0, c1 = 0, c2 = 0, c3 = 0;
	int error;

	vib_enable(1);
	if ((error = qup_i2c_regread(bus, DRV_ADDR, DRV_STATUS, &st)) != 0) {
		vib_enable(0);
		return (error);
	}
	dw(DRV_MODE, 0);				/* ready */
	dw(DRV_RATED_VOLT, 0x30);
	dw(DRV_OD_CLAMP, 0x40);
	qup_i2c_regread(bus, DRV_ADDR, DRV_FEEDBACK, &fb);
	qup_i2c_regread(bus, DRV_ADDR, DRV_CTRL1, &c1);
	qup_i2c_regread(bus, DRV_ADDR, DRV_CTRL2, &c2);
	qup_i2c_regread(bus, DRV_ADDR, DRV_CTRL3, &c3);
	dw(DRV_FEEDBACK, (fb & 0x7c) | 0x80 | 0x01);	/* LRA, BEMF gain 1 */
	dw(DRV_CTRL1, (c1 & ~0x1f) | (5 * (1000 - 240) / 240));	/* drive time */
	dw(DRV_CTRL2, c2 | 0x80);			/* bidirectional input */
	dw(DRV_CTRL3, c3 & ~(0x20 | 0x08 | 0x01));	/* closed loop, signed */
	dw(DRV_CAL_COMP, 0x08);
	dw(DRV_CAL_BEMF, 0x8e);
	dw(DRV_RTP_IN, 0x7f);
	dw(DRV_MODE, MODE_STANDBY);
	vib_enable(0);
	st9("vibrator: DRV260x id %d ready (was feedback 0x%02x ctrl "
	    "0x%02x/0x%02x/0x%02x)\n", st >> 5, fb, c1, c2, c3);
	return (0);
}

static void
vib_stop(void *arg __unused, int pending __unused)
{
	dw(DRV_MODE, MODE_STANDBY);
	vib_enable(0);
}

static void
vib_stop_timeout(void *arg __unused)
{
	taskqueue_enqueue(taskqueue_thread[0], &vib_stop_task);
}

static int
vib_start(int ms)
{
	int error;

	if (ms <= 0) {
		callout_stop(&vib_stop_co);
		vib_stop(NULL, 0);
		return (0);
	}
	if (ms > 5000)
		ms = 5000;
	vib_enable(1);
	if ((error = dw(DRV_RTP_IN, 0x7f)) == 0)
		error = dw(DRV_MODE, MODE_RTP);
	callout_reset(&vib_stop_co, (ms * hz + 999) / 1000, vib_stop_timeout,
	    NULL);
	return (error);
}

static int
sysctl_vibrate(SYSCTL_HANDLER_ARGS)
{
	int ms = 0, error;

	error = sysctl_handle_int(oidp, &ms, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	if (!vib_ok)
		return (ENXIO);
	return (vib_start(ms));
}

/* ---- key backlight ---- */

/* PM8994 GPIO 7 as a digital output at (level). */
static int
pmgpio7_out(int level)
{
	int error;

	error = msm8996_spmi_write(0, PMGPIO7 + PMGPIO_MODE_CTL,
	    (1 << 4) | (level ? 1 : 0));
	if (error == 0)
		error = msm8996_spmi_write(0, PMGPIO7 + PMGPIO_DIG_OUT, 0x02);
	if (error == 0)
		error = msm8996_spmi_write(0, PMGPIO7 + PMGPIO_EN_CTL, 0x80);
	return (error);
}

static int
lw(uint8_t reg, uint8_t v)
{
	return (qup_i2c_regwrite(bus, LP_ADDR, reg, v));
}

static int
lp_set(int level)
{
	int error;

	if (level < 0)
		level = 0;
	if (level > 255)
		level = 255;
	error = lw(LP_R_PWM, level);
	lw(LP_G_PWM, level);
	lw(LP_B_PWM, level);
	lw(LP_W_PWM, level);
	if (error == 0)
		keylight = level;
	return (error);
}

static int
lp_init(void)
{
	uint8_t v;
	int error, i;

	error = qup_i2c_regread(bus, LP_ADDR, LP_ENABLE, &v);
	if (error) {
		uint8_t m = 0, o = 0, e = 0;

		msm8996_spmi_read(0, PMGPIO7 + PMGPIO_MODE_CTL, &m);
		msm8996_spmi_read(0, PMGPIO7 + PMGPIO_DIG_OUT, &o);
		msm8996_spmi_read(0, PMGPIO7 + PMGPIO_EN_CTL, &e);
		st9("keylight: no answer (error %d); PM8994 GPIO 7 mode "
		    "0x%02x out 0x%02x en 0x%02x, owner ee%d\n", error, m, o, e,
		    msm8996_spmi_owner(0, PMGPIO7));
	}
	/* No answer: the enable line, active low in the device tree. */
	for (i = 0; error == ENXIO && i < 2; ++i) {
		int e2 = pmgpio7_out(i);

		if (e2 != 0) {
			st9("keylight: PM8994 GPIO 7 write: error %d\n",
			    e2);
			break;
		}
		tsleep(&lp_ok, 0, "lpen", hz / 100 + 1);
		error = qup_i2c_regread(bus, LP_ADDR, LP_ENABLE, &v);
		if (error == 0)
			st9("keylight: enabled with PM8994 GPIO 7 %s\n",
			    i ? "high" : "low");
	}
	if (error)
		return (error);
	lw(LP_RESET, 0xff);
	tsleep(&lp_ok, 0, "lprst", hz / 100 + 1);
	lw(LP_ENABLE, LP_MASTER_EN | LP_LOG_PWM);
	tsleep(&lp_ok, 0, "lpen", hz / 100 + 1);
	lw(LP_OP_MODE, LP_DIRECT);
	DELAY(200);
	lw(LP_CONFIG, LP_CFG_DEFAULT);
	lw(LP_R_CUR, LP_CURRENT);
	lw(LP_G_CUR, LP_CURRENT);
	lw(LP_B_CUR, LP_CURRENT);
	lw(LP_W_CUR, LP_CURRENT);
	lw(LP_ENG_SEL, 0);
	return (lp_set(0));
}

static int
sysctl_keylight(SYSCTL_HANDLER_ARGS)
{
	int v = keylight, error;

	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	if (!lp_ok)
		return (ENXIO);
	return (lp_set(v));
}

SYSCTL_DECL(_hw_mi5);
SYSCTL_PROC(_hw_mi5, OID_AUTO, vibrate, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_vibrate, "I", "vibrate for this many milliseconds");
SYSCTL_PROC(_hw_mi5, OID_AUTO, keylight, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_keylight, "I", "key backlight, 0-255");

static void
i2c9_thread(void *arg __unused)
{
	static const struct { int a; const char *n; } devs[] = {
		{ 0x34, "speaker amplifier" }, { 0x47, "USB-C (tusb320)" },
		{ 0x67, "USB-C (tusb320)" },
	};
	uint8_t v;
	int error, i;

	st9("i2c9: thread started");
	if ((bus = qup_i2c_bus(9)) == NULL) {
		st9("i2c9: not available\n");
		goto out;
	}
	for (i = 0; i < (int)NELEM(devs); ++i) {
		error = qup_i2c_regread(bus, devs[i].a, 0, &v);
		st9("i2c9: 0x%02x %s: %s\n", devs[i].a, devs[i].n,
		    error == 0 ? "answers" : error == ENXIO ? "no answer" :
		    "bus error");
	}
	error = vib_init();
	vib_ok = error == 0;
	if (error)
		st9("vibrator: no answer at 0x%02x (error %d)\n", DRV_ADDR,
		    error);
	error = lp_init();
	lp_ok = error == 0;
	st9("keylight: LP5562 %s\n", lp_ok ? "ready" : "not answering");
	if (vib_ok)
		vib_start(80);		/* a short buzz: the driver works */
	if (lp_ok)
		lp_set(64);
out:
	kthread_exit();
}

static void
i2c9_init(void *dummy __unused)
{
	struct thread *td;
	int enable = 1;

	if (!msm8996_is_present())
		return;
	TUNABLE_INT_FETCH("msm8996.i2c9", &enable);
	if (!enable)
		return;
	callout_init_mp(&vib_stop_co);
	TASK_INIT(&vib_stop_task, 0, vib_stop, NULL);
	kthread_create(i2c9_thread, NULL, &td, "i2c9");
}
SYSINIT(msm8996i2c9, SI_SUB_CONFIGURE + 1, SI_ORDER_ANY, i2c9_init, NULL);
