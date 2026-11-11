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
 * The Mi 5's buttons, polled.
 *
 *	power		PM8994 power-on block (PON) at 0x800, KPDPWR
 *	volume down	PM8994 PON, RESIN
 *	volume up	PM8994 GPIO 2 (0xc100), active low
 *	home		SoC (TLMM) GPIO 34, active low
 *
 * The PMIC is reached over SPMI through Qualcomm's PMIC arbiter.  No BSD
 * has a driver for it; the register protocol here follows Linux's
 * drivers/spmi/spmi-pmic-arb.c (version 2 arbiter, as on MSM8996), read
 * side only: find the peripheral's channel (APID) in the core's channel
 * map, write a read command to that channel's observer registers, wait
 * for DONE, take RDATA0.  Nothing is written to the PMIC.
 *
 * A kernel thread samples the keys every 20ms and reports changes on the
 * console; key_events() hands them to whoever wants them (the console
 * line editor, later evdev).  Boot argument msm8996.keys=0 skips it.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kthread.h>

#include <dev/misc/evdev/input.h>
#include <dev/misc/evdev/evdev.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

/* Arbiter regions, from the device tree (qcom,spmi@400f000) */
#define	ARB_CORE	0x0400f000UL
#define	ARB_CORE_SIZE	0xc00
#define	ARB_OBSRVR	0x04c00000UL
#define	ARB_EE		0

#define	ARB_CHNL_MAP(n)	(0x800 + 4 * (n))	/* v2: PMIC_ARB_REG_CHNLn */
#define	ARB_CMD		0x00
#define	ARB_STATUS	0x08
#define	  ST_DONE	(1U << 0)
#define	  ST_FAILURE	(1U << 1)
#define	  ST_DENIED	(1U << 2)
#define	  ST_DROPPED	(1U << 3)
#define	ARB_RDATA0	0x18
#define	OP_EXT_READL	1

/* PM8994 (SPMI slave 0) */
#define	PON_BASE	0x0800
#define	PON_RT_STS	0x10
#define	  PON_KPDPWR	(1U << 0)
#define	  PON_RESIN	(1U << 1)
#define	GPIO2_BASE	0xc100
#define	GPIO_STATUS1	0x08
#define	GPIO_RT_STS	0x10

/* TLMM */
#define	TLMM_BASE	0x01010000UL
#define	TLMM_IN_OUT(n)	(TLMM_BASE + 0x1000 * (n) + 4)
#define	HOME_GPIO	34

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

/* Channel (APID) for a peripheral, or -1. */
static int
spmi_apid(int sid, uint16_t addr)
{
	uint32_t ppid = (sid << 8) | ((addr >> 8) & 0xff);
	uint32_t v;
	int n;

	for (n = 0; ARB_CHNL_MAP(n) < ARB_CORE_SIZE; ++n) {
		v = rd(ARB_CORE + ARB_CHNL_MAP(n));
		if (v != 0 && ((v >> 8) & 0xfff) == ppid)
			return (n);
	}
	return (-1);
}

/* One byte from (addr) of the peripheral on channel (apid). */
static int
spmi_read(int apid, uint16_t addr, uint8_t *val)
{
	vm_paddr_t ch = ARB_OBSRVR + 0x1000 * ARB_EE + 0x8000 * apid;
	uint32_t st = 0;
	int i;

	wr(ch + ARB_CMD, (OP_EXT_READL << 27) | ((addr & 0xff) << 4) | 0);
	for (i = 0; i < 1000; ++i) {
		st = rd(ch + ARB_STATUS);
		if (st & ST_DONE)
			break;
		DELAY(1);
	}
	if ((st & ST_DONE) == 0 || (st & (ST_FAILURE | ST_DENIED | ST_DROPPED)))
		return (EIO);
	*val = rd(ch + ARB_RDATA0) & 0xff;
	return (0);
}

static int pon_apid = -1, gpio2_apid = -1;

#define	KB_POWER	0x01
#define	KB_VOLDOWN	0x02
#define	KB_VOLUP	0x04
#define	KB_HOME	0x08

static const char *const key_names[] = {
	"power", "volume down", "volume up", "home"
};

/* Linux input codes, as the device tree's linux,code properties give. */
static const uint16_t key_codes[] = {
	KEY_POWER, KEY_VOLUMEDOWN, KEY_VOLUMEUP, KEY_HOME
};

static struct evdev_dev *keys_evdev;	/* /dev/input/eventN */
static int keys_verbose = 1;

static u_int
keys_sample(void)
{
	u_int k = 0;
	uint8_t v;

	if (pon_apid >= 0 && spmi_read(pon_apid, PON_BASE + PON_RT_STS, &v) == 0) {
		if (v & PON_KPDPWR)
			k |= KB_POWER;
		if (v & PON_RESIN)
			k |= KB_VOLDOWN;
	}
	if (gpio2_apid >= 0 &&
	    spmi_read(gpio2_apid, GPIO2_BASE + GPIO_RT_STS, &v) == 0 &&
	    (v & 1) == 0)
		k |= KB_VOLUP;
	if ((rd(TLMM_IN_OUT(HOME_GPIO)) & 1) == 0)
		k |= KB_HOME;
	return (k);
}

/* Last stable state, for the console's use. */
static volatile u_int keys_down;
static volatile u_int keys_pressed_count;

u_int	msm8996_keys_down(void);

u_int
msm8996_keys_down(void)
{
	return (keys_down);
}

static void
keys_thread(void *arg __unused)
{
	u_int last, now, prev, changed;
	int i;

	last = prev = keys_sample();
	keys_down = last;
	for (;;) {
		tsleep(&keys_thread, 0, "keys", hz / 50 ? hz / 50 : 1);
		now = keys_sample();
		/* Debounce: a change must be seen on two samples in a row. */
		if (now != prev) {
			prev = now;
			continue;
		}
		changed = now ^ last;
		if (changed == 0)
			continue;
		for (i = 0; i < 4; ++i) {
			if (changed & (1U << i)) {
				if (keys_verbose)
					kprintf("key: %s %s\n", key_names[i],
					    (now & (1U << i)) ? "pressed" :
					    "released");
				if (keys_evdev != NULL)
					evdev_push_key(keys_evdev, key_codes[i],
					    (now & (1U << i)) != 0);
				if (now & (1U << i))
					++keys_pressed_count;
			}
		}
		if (keys_evdev != NULL)
			evdev_sync(keys_evdev);
		last = now;
		keys_down = now;
	}
}

static void
msm8996_keys_init(void *dummy __unused)
{
	struct thread *td;
	uint8_t pon = 0, g1 = 0, g2 = 0;
	int enable = 1;

	if (!msm8996_is_present())
		return;
	TUNABLE_INT_FETCH("msm8996.keys", &enable);
	if (!enable)
		return;

	pon_apid = spmi_apid(0, PON_BASE);
	gpio2_apid = spmi_apid(0, GPIO2_BASE);
	if (pon_apid >= 0)
		spmi_read(pon_apid, PON_BASE + PON_RT_STS, &pon);
	if (gpio2_apid >= 0) {
		spmi_read(gpio2_apid, GPIO2_BASE + GPIO_STATUS1, &g1);
		spmi_read(gpio2_apid, GPIO2_BASE + GPIO_RT_STS, &g2);
	}
	kprintf("keys: spmi arbiter 0x%x, PON channel %d (status 0x%02x), "
	    "PMIC GPIO2 channel %d (0x%02x 0x%02x), home GPIO %d = %d\n",
	    rd(ARB_CORE), pon_apid, pon, gpio2_apid, g1, g2, HOME_GPIO,
	    rd(TLMM_IN_OUT(HOME_GPIO)) & 1);
	TUNABLE_INT_FETCH("msm8996.keys_verbose", &keys_verbose);

	/* An evdev device, the interface Arcan and others read. */
	keys_evdev = evdev_alloc();
	evdev_set_name(keys_evdev, "Mi 5 buttons");
	evdev_set_phys(keys_evdev, "pm8994-pon+gpio");
	evdev_set_id(keys_evdev, BUS_HOST, 0, 0, 0);
	evdev_support_event(keys_evdev, EV_SYN);
	evdev_support_event(keys_evdev, EV_KEY);
	for (int i = 0; i < 4; ++i)
		evdev_support_key(keys_evdev, key_codes[i]);
	if (evdev_register(keys_evdev) != 0) {
		evdev_free(keys_evdev);
		keys_evdev = NULL;
	}
	kthread_create(keys_thread, NULL, &td, "keys");
}
SYSINIT(msm8996keys, SI_SUB_CONFIGURE, SI_ORDER_ANY, msm8996_keys_init, NULL);
