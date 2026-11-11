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
 * MSM8996's SPMI PMIC arbiter (version 2): one-byte register reads and
 * writes on the PMICs (PM8994 at SPMI slaves 0/1, PMI8994 at 2/3).
 *
 * The register protocol follows Linux's drivers/spmi/spmi-pmic-arb.c:
 * a peripheral (slave, address bits 15:8) is reached through a channel
 * (APID) found in the core's channel map; reads go through the observer
 * registers, writes through the channel registers, each a command word
 * then a wait for DONE.  The arbiter only accepts writes from the
 * execution environment that owns a peripheral (the ownership table in
 * the cnfg region), so writes to anything not owned by the application
 * processor (EE 0) are refused here before the hardware refuses them.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

/* Regions, from the device tree (qcom,spmi@400f000) */
#define	ARB_CORE	0x0400f000UL
#define	ARB_CORE_SIZE	0xc00
#define	ARB_CHNLS	0x04400000UL
#define	ARB_OBSRVR	0x04c00000UL
#define	ARB_CNFG	0x0400a000UL
#define	ARB_EE		0

#define	ARB_CHNL_MAP(n)	(0x800 + 4 * (n))
#define	ARB_OWNER(n)	(0x700 + 4 * (n))	/* in cnfg */
#define	ARB_CMD		0x00
#define	ARB_STATUS	0x08
#define	  ST_DONE	(1U << 0)
#define	  ST_FAILURE	(1U << 1)
#define	  ST_DENIED	(1U << 2)
#define	  ST_DROPPED	(1U << 3)
#define	ARB_WDATA0	0x10
#define	ARB_RDATA0	0x18
#define	OP_EXT_WRITEL	0
#define	OP_EXT_READL	1

#define	MAXAPID		((ARB_CORE_SIZE - 0x800) / 4)

static struct lock spmi_lock;
static int16_t apid_cache[16][256];	/* [sid][addr >> 8]: apid + 1, or -1 */
static int spmi_inited;

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

static void
spmi_init(void)
{
	if (spmi_inited)
		return;
	lockinit(&spmi_lock, "spmi", 0, 0);
	spmi_inited = 1;
}

/* Channel (APID) for a peripheral, or -1. */
int
msm8996_spmi_apid(int sid, uint16_t addr)
{
	uint32_t ppid = (sid << 8) | ((addr >> 8) & 0xff);
	int16_t *c = &apid_cache[sid & 15][(addr >> 8) & 0xff];
	uint32_t v;
	int n;

	if (!msm8996_is_present())
		return (-1);
	if (*c != 0)
		return (*c > 0 ? *c - 1 : -1);
	for (n = 0; n < MAXAPID; ++n) {
		v = rd(ARB_CORE + ARB_CHNL_MAP(n));
		if (v != 0 && ((v >> 8) & 0xfff) == ppid) {
			*c = n + 1;
			return (n);
		}
	}
	*c = -1;
	return (-1);
}

/* Which execution environment may write the peripheral (0: us). */
int
msm8996_spmi_owner(int sid, uint16_t addr)
{
	int apid = msm8996_spmi_apid(sid, addr);

	if (apid < 0)
		return (-1);
	return (rd(ARB_CNFG + ARB_OWNER(apid)) & 7);
}

static int
spmi_wait(vm_paddr_t ch)
{
	uint32_t st = 0;
	int i;

	for (i = 0; i < 1000; ++i) {
		st = rd(ch + ARB_STATUS);
		if (st & ST_DONE)
			break;
		DELAY(1);
	}
	if ((st & ST_DONE) == 0)
		return (ETIMEDOUT);
	if (st & ST_DENIED)
		return (EPERM);
	if (st & (ST_FAILURE | ST_DROPPED))
		return (EIO);
	return (0);
}

int
msm8996_spmi_read(int sid, uint16_t addr, uint8_t *val)
{
	int apid = msm8996_spmi_apid(sid, addr);
	vm_paddr_t ch;
	int error;

	if (apid < 0)
		return (ENXIO);
	spmi_init();
	ch = ARB_OBSRVR + 0x1000 * ARB_EE + 0x8000 * apid;
	lockmgr(&spmi_lock, LK_EXCLUSIVE);
	wr(ch + ARB_CMD, (OP_EXT_READL << 27) | ((addr & 0xff) << 4) | 0);
	error = spmi_wait(ch);
	if (error == 0)
		*val = rd(ch + ARB_RDATA0) & 0xff;
	lockmgr(&spmi_lock, LK_RELEASE);
	return (error);
}

int
msm8996_spmi_write(int sid, uint16_t addr, uint8_t val)
{
	int apid = msm8996_spmi_apid(sid, addr);
	vm_paddr_t ch;
	int error;

	if (apid < 0)
		return (ENXIO);
	if ((rd(ARB_CNFG + ARB_OWNER(apid)) & 7) != ARB_EE)
		return (EPERM);
	spmi_init();
	ch = ARB_CHNLS + 0x1000 * ARB_EE + 0x8000 * apid;
	lockmgr(&spmi_lock, LK_EXCLUSIVE);
	wr(ch + ARB_WDATA0, val);
	wr(ch + ARB_CMD, (OP_EXT_WRITEL << 27) | ((addr & 0xff) << 4) | 0);
	error = spmi_wait(ch);
	lockmgr(&spmi_lock, LK_RELEASE);
	return (error);
}

/* Read-modify-write of the bits in (mask). */
int
msm8996_spmi_update(int sid, uint16_t addr, uint8_t mask, uint8_t val)
{
	uint8_t v;
	int error;

	if ((error = msm8996_spmi_read(sid, addr, &v)) != 0)
		return (error);
	return (msm8996_spmi_write(sid, addr, (v & ~mask) | (val & mask)));
}

static void
spmi_sysinit(void *dummy __unused)
{
	spmi_init();
}
SYSINIT(msm8996spmi, SI_SUB_CONFIGURE, SI_ORDER_FIRST, spmi_sysinit, NULL);
