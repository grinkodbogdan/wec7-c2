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
 * The PMI8994's parts the Mi 5 uses, over SPMI (qcom_spmi.c):
 *
 *	hw.mi5.backlight	display backlight, 0-4095 (WLED, slave 3,
 *				0xd800 control, 0xd900 sinks, strings 0-1)
 *	hw.mi5.led		notification LED, 0xRRGGBB (LPG channels
 *				0xb100-0xb300 through the TRILED at 0xd000)
 *	hw.mi5.battery		charge, percent (fuel gauge, slave 2, 0x4000)
 *	hw.mi5.charger		"none", "present", "charging", "full"
 *				(SMBCHG, slave 2, 0x1000 and 0x1300)
 *
 * Register sequences after Linux: drivers/video/backlight/qcom-wled.c
 * (WLED4), drivers/leds/rgb/leds-qcom-lpg.c (PMI8994 LPG and TRILED),
 * and the Mi 5's downstream qpnp-fg.c and qpnp-smbcharger.c (monotonic
 * state of charge; charger status bits).  No code is taken from them.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/sysctl.h>

#include <machine/md_var.h>

#define	PMI_SID0	2		/* charger, fuel gauge */
#define	PMI_SID1	3		/* WLED, LPG, TRILED */

/* WLED (WLED4) */
#define	WLED_CTRL	0xd800
#define	  WLED_MOD_EN		0x46
#define	  WLED_MOD_EN_BIT	0x80
#define	WLED_SINK	0xd900
#define	  WLED_SYNC		0x47
#define	  WLED_BRIGHT(n)	(0x57 + 0x10 * (n))
#define	WLED_MAX	4095
#define	WLED_STRINGS	2

/* LPG channels and the TRILED */
#define	LPG_CH(n)	(0xb100 + 0x100 * (n))
#define	  LPG_SIZE_CLK		0x41
#define	  LPG_PREDIV_CLK	0x42
#define	  LPG_TYPE_CONFIG	0x43
#define	    LPG_GLITCH_REMOVAL	0x20
#define	  LPG_PWM_VALUE		0x44	/* and 0x45 */
#define	  LPG_ENABLE_CONTROL	0x46
#define	    LPG_EN_OUTPUT	0x80
#define	    LPG_EN_TRISTATE	0x20
#define	    LPG_EN_SRC_PWM	0x04
#define	  LPG_SYNC		0x47
#define	TRILED		0xd000
#define	  TRILED_SRC_SEL	0x45
#define	  TRILED_EN_CTL		0x46
#define	  TRILED_ATC_CTL	0x47

/* Fuel gauge */
#define	FG_SOC		0x4000
#define	  FG_MONOTONIC_SOC	0x09	/* and its shadow at 0x0a */

/* Charger */
#define	CHGR		0x1000
#define	  CHGR_STS		0x0e
#define	    CHG_TYPE(r)		(((r) >> 1) & 3)
#define	  CHGR_RT_STS		0x10
#define	    CHG_INHIBIT		0x02
#define	    BAT_TCC_REACHED	0x80
#define	USB_CHGPTH	0x1300
#define	  USB_RT_STS		0x10
#define	    USBIN_OV		0x02
#define	    USBIN_SRC_DET	0x04

static int backlight = -1;
static uint32_t led_rgb;

static int
wr3(uint16_t a, uint8_t v)
{
	return (msm8996_spmi_write(PMI_SID1, a, v));
}

/* ---- backlight ---- */

static int
wled_set(int level)
{
	int i, error = 0;

	if (level < 0)
		level = 0;
	if (level > WLED_MAX)
		level = WLED_MAX;
	if (level > 0 && level < WLED_MAX * 4 / 1000)
		level = WLED_MAX * 4 / 1000;	/* WLED4's lower limit */
	for (i = 0; i < WLED_STRINGS && error == 0; ++i) {
		error = wr3(WLED_SINK + WLED_BRIGHT(i), level & 0xff);
		if (error == 0)
			error = wr3(WLED_SINK + WLED_BRIGHT(i) + 1, level >> 8);
	}
	if (error == 0 && level > 0) {
		/* latch the new values: sync bits down, then up */
		error = msm8996_spmi_update(PMI_SID1, WLED_SINK + WLED_SYNC,
		    0x0f, 0);
		if (error == 0)
			error = msm8996_spmi_update(PMI_SID1,
			    WLED_SINK + WLED_SYNC, 0x0f, 0x0f);
	}
	if (error == 0)
		error = msm8996_spmi_update(PMI_SID1, WLED_CTRL + WLED_MOD_EN,
		    WLED_MOD_EN_BIT, level ? WLED_MOD_EN_BIT : 0);
	if (error == 0)
		backlight = level;
	return (error);
}

static int
wled_get(void)
{
	uint8_t lo, hi, en;

	if (msm8996_spmi_read(PMI_SID1, WLED_SINK + WLED_BRIGHT(0), &lo) ||
	    msm8996_spmi_read(PMI_SID1, WLED_SINK + WLED_BRIGHT(0) + 1, &hi) ||
	    msm8996_spmi_read(PMI_SID1, WLED_CTRL + WLED_MOD_EN, &en))
		return (-1);
	if ((en & WLED_MOD_EN_BIT) == 0)
		return (0);
	return (((hi & 0x0f) << 8) | lo);
}

static int
sysctl_backlight(SYSCTL_HANDLER_ARGS)
{
	int v, error;

	v = wled_get();
	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	return (wled_set(v));
}

/* ---- notification LED ---- */

/*
 * One LPG channel as a plain PWM output at duty (0..511 of 9 bits),
 * after lpg_apply(): size and clock, divider, value, enable, sync.
 */
static int
lpg_set(int ch, int duty)
{
	uint16_t b = LPG_CH(ch);
	int error;

	error = msm8996_spmi_update(PMI_SID1, b + LPG_TYPE_CONFIG,
	    LPG_GLITCH_REMOVAL, LPG_GLITCH_REMOVAL);
	if (error)
		return (error);
	wr3(b + LPG_SIZE_CLK, 0x30 | 1);	/* 9 bits, 1.024kHz clock */
	wr3(b + LPG_PREDIV_CLK, 0);
	wr3(b + LPG_PWM_VALUE, duty & 0xff);
	wr3(b + LPG_PWM_VALUE + 1, duty >> 8);
	wr3(b + LPG_ENABLE_CONTROL, LPG_EN_TRISTATE | LPG_EN_SRC_PWM |
	    (duty ? LPG_EN_OUTPUT : 0));
	wr3(b + LPG_PWM_VALUE, duty & 0xff);	/* again: a hardware bug */
	wr3(b + LPG_PWM_VALUE + 1, duty >> 8);
	wr3(b + LPG_SYNC, 1);
	return (msm8996_spmi_update(PMI_SID1, b + LPG_TYPE_CONFIG,
	    LPG_GLITCH_REMOVAL, 0));
}

/*
 * Channel 0 (0xb100) drives TRILED bit 5, 1 bit 6, 2 bit 7 (Linux's
 * pmi8994 table).  Bit 7 is red in the downstream driver, so: blue,
 * green, red.
 */
static const int led_chan[3] = { 2, 1, 0 };	/* R, G, B */

static int
led_set(uint32_t rgb)
{
	uint8_t en = 0;
	int i, c, error;

	wr3(TRILED + TRILED_ATC_CTL, 0);
	wr3(TRILED + TRILED_SRC_SEL, 1);		/* VPH_PWR */
	for (i = 0; i < 3; ++i) {
		c = (rgb >> (16 - 8 * i)) & 0xff;
		if ((error = lpg_set(led_chan[i], c * 511 / 255)) != 0)
			return (error);
		if (c)
			en |= 0x20 << led_chan[i];
	}
	error = msm8996_spmi_update(PMI_SID1, TRILED + TRILED_EN_CTL, 0xe0, en);
	if (error == 0)
		led_rgb = rgb;
	return (error);
}

static int
sysctl_led(SYSCTL_HANDLER_ARGS)
{
	int v = led_rgb, error;

	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	return (led_set(v & 0xffffff));
}

/* ---- battery and charger ---- */

static int
battery_percent(void)
{
	uint8_t a, b;
	int tries;

	for (tries = 0; tries < 5; ++tries) {
		if (msm8996_spmi_read(PMI_SID0, FG_SOC + FG_MONOTONIC_SOC, &a) ||
		    msm8996_spmi_read(PMI_SID0, FG_SOC + FG_MONOTONIC_SOC + 1,
		    &b))
			return (-1);
		if (a == b)
			break;
	}
	if (a != b)
		return (-1);
	if (a == 0xff)
		return (100);
	if (a == 0)
		return (0);
	return (((a - 1) * 98 + 126) / 253 + 1);
}

static const char *
charger_state(void)
{
	uint8_t usb, rt, sts;

	if (msm8996_spmi_read(PMI_SID0, USB_CHGPTH + USB_RT_STS, &usb))
		return ("unknown");
	if ((usb & USBIN_SRC_DET) == 0 || (usb & USBIN_OV))
		return ("none");
	if (msm8996_spmi_read(PMI_SID0, CHGR + CHGR_RT_STS, &rt) ||
	    msm8996_spmi_read(PMI_SID0, CHGR + CHGR_STS, &sts))
		return ("present");
	if (rt & (BAT_TCC_REACHED | CHG_INHIBIT))
		return ("full");
	return (CHG_TYPE(sts) ? "charging" : "present");
}

static int
sysctl_battery(SYSCTL_HANDLER_ARGS)
{
	int v = battery_percent();

	return (sysctl_handle_int(oidp, &v, 0, req));
}

static int
sysctl_charger(SYSCTL_HANDLER_ARGS)
{
	const char *s = charger_state();

	return (sysctl_handle_string(oidp, __DECONST(char *, s),
	    strlen(s) + 1, req));
}

SYSCTL_NODE(_hw, OID_AUTO, mi5, CTLFLAG_RW, 0, "Xiaomi Mi 5 hardware");
SYSCTL_PROC(_hw_mi5, OID_AUTO, backlight, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_backlight, "I", "display backlight, 0-4095");
SYSCTL_PROC(_hw_mi5, OID_AUTO, led, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_led, "IX", "notification LED, 0xRRGGBB");
SYSCTL_PROC(_hw_mi5, OID_AUTO, battery, CTLTYPE_INT | CTLFLAG_RD, 0, 0,
    sysctl_battery, "I", "battery charge, percent");
SYSCTL_PROC(_hw_mi5, OID_AUTO, charger, CTLTYPE_STRING | CTLFLAG_RD, 0, 0,
    sysctl_charger, "A", "charger state");

static void
pmi8994_init(void *dummy __unused)
{
	static const struct { int sid; uint16_t a; const char *n; } parts[] = {
		{ PMI_SID1, WLED_CTRL, "wled" },
		{ PMI_SID1, WLED_SINK, "wled sinks" },
		{ PMI_SID1, LPG_CH(0), "lpg" },
		{ PMI_SID1, TRILED, "triled" },
		{ PMI_SID0, FG_SOC, "fuel gauge" },
		{ PMI_SID0, CHGR, "charger" },
	};
	int i;

	if (!msm8996_is_present())
		return;
	kprintf("pmi8994:");
	for (i = 0; i < (int)NELEM(parts); ++i) {
		int apid = msm8996_spmi_apid(parts[i].sid, parts[i].a);

		if (apid < 0)
			kprintf(" %s -", parts[i].n);
		else
			kprintf(" %s ch%d/ee%d", parts[i].n, apid,
			    msm8996_spmi_owner(parts[i].sid, parts[i].a));
	}
	kprintf("\npmi8994: backlight %d, battery %d%%, charger %s\n",
	    wled_get(), battery_percent(), charger_state());
}
SYSINIT(pmi8994, SI_SUB_CONFIGURE, SI_ORDER_ANY, pmi8994_init, NULL);
