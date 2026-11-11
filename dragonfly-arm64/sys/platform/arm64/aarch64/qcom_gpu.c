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
 * Adreno 530 bring-up, first step: power.
 *
 * Before the GPU's registers can be read, its rail, power domains and
 * clocks must be on; an access to an unclocked GPU hangs the bus.  This
 * reports their state as the bootloader left it, then turns them on in
 * the order Linux does (drivers/clk/qcom/mmcc-msm8996.c, gdsc.c, the
 * msm8996 device tree), checking each step, and only then reads a few
 * GPU and GPU-SMMU registers.  The GPU driver proper (Linux's drm/msm
 * adreno code) comes next, in its own GPL directory.
 *
 *	vdd_gfx		PMI8994 S2 (SPMI slave 3, 0x1700)
 *	gpu gdsc	MMCC 0x4034 (status in 0x4038), the always-on part
 *	gpu_gx gdsc	MMCC 0x4024, clamp at 0x4300, needs vdd_gfx
 *	clocks		gcc_mmss_bimc_gfx, gcc_bimc_gfx, gpu_ahb,
 *			gpu_gx_gfx3d, gpu_gx_rbbmtimer
 *	GPU		0xb00000, SMMU 0xb40000
 *
 * Boot argument msm8996.gpu: 0 skips this, 1 reports only, 2 (the
 * default) powers up.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/sysctl.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	GCC_BASE		0x00300000UL
#define	GCC_MMSS_BIMC_GFX	(GCC_BASE + 0x9010)
#define	GCC_BIMC_GFX		(GCC_BASE + 0x46018)

#define	MMCC_BASE		0x008c0000UL
#define	MM(o)			(MMCC_BASE + (o))
#define	GFX3D_RCG		MM(0x4000)
#define	GPU_GX_GDSCR		MM(0x4024)
#define	GPU_GX_GFX3D_CBCR	MM(0x4028)
#define	GPU_GDSCR		MM(0x4034)
#define	GPU_GDS_HW_CTRL		MM(0x4038)
#define	GPU_AHB_CBCR		MM(0x403c)
#define	RBBMTIMER_RCG		MM(0x4090)
#define	GPU_RBBMTIMER_CBCR	MM(0x40b0)
#define	GPU_GX_CLAMP_IO		MM(0x4300)

#define	CBCR_EN			(1U << 0)
#define	CBCR_OFF		(1U << 31)
#define	GDSC_SW_COLLAPSE	(1U << 0)
#define	GDSC_PWR_ON		(1U << 31)

#define	GPU_BASE		0x00b00000UL
#define	GPU_REG(dw)		(GPU_BASE + 4 * (dw))
#define	A5XX_RBBM_STATUS	0x04f5
#define	A5XX_RBBM_STATUS3	0x0530
#define	A5XX_RBBM_INT_0_STATUS	0x04e1
#define	A5XX_CP_PROTECT_STATUS	0x0b1c
#define	A5XX_RBBM_SECVID_TRUST	0xf400
#define	SMMU_BASE		0x00b40000UL
#define	SMMU_SCR0		0x000
#define	SMMU_IDR0		0x020
#define	SMMU_IDR1		0x024

/* PMI8994 S2, an FTSMPS */
#define	S2_SID			3
#define	S2_BASE			0x1700
#define	  REG_TYPE		0x04
#define	  REG_SUBTYPE		0x05
#define	  REG_RANGE		0x40
#define	  REG_VSET		0x41
#define	  REG_EN		0x46
#define	    EN_BIT		0x80

/*
 * Progress, readable with sysctl hw.mi5.gpu whatever happens to kernel
 * messages, and showing the step reached if one hangs.
 */
static char gpu_status[2048] = "not started";
static size_t gpu_status_len;
SYSCTL_DECL(_hw_mi5);
SYSCTL_STRING(_hw_mi5, OID_AUTO, gpu, CTLFLAG_RD, gpu_status, 0,
    "GPU bring-up progress");

/* One more line of progress (the last one is where it stopped). */
static void
status(const char *fmt, ...)
{
	char line[256];
	__va_list ap;
	size_t n;

	__va_start(ap, fmt);
	kvsnprintf(line, sizeof(line), fmt, ap);
	__va_end(ap);
	n = strlen(line);
	if (gpu_status_len + n + 2 < sizeof(gpu_status)) {
		gpu_status[gpu_status_len++] = '\n';
		memcpy(gpu_status + gpu_status_len, line, n + 1);
		gpu_status_len += n;
	}
	cpu_sfence();
	kprintf("gpu: %s\n", line);
}

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

/* Wait for (pa & mask) == want; 0 or ETIMEDOUT. */
static int
waitbits(vm_paddr_t pa, uint32_t mask, uint32_t want, int us)
{
	while (us-- > 0) {
		if ((rd(pa) & mask) == want)
			return (0);
		DELAY(1);
	}
	return (ETIMEDOUT);
}

static int
clk_on(const char *name, vm_paddr_t cbcr)
{
	wr(cbcr, rd(cbcr) | CBCR_EN);
	if (waitbits(cbcr, CBCR_OFF, 0, 2000)) {
		status("clock %s does not start (0x%08x)", name,
		    rd(cbcr));
		return (EIO);
	}
	return (0);
}

/*
 * vdd_gfx in microvolts, or -1: min_uV + selector * step for the range
 * (Linux's spmi_hw_selector_to_sw with the ftsmps / ftsmps2p5 tables;
 * the Mi 5's S2 is subtype 0x09, FTS2.5).
 */
static int
s2_uv(uint8_t sub, uint8_t range, uint8_t vset)
{
	if (sub == 0x09)		/* FTS2.5 */
		return (range == 0 ? 80000 + 5000 * vset :
		    range == 1 ? 160000 + 10000 * vset : -1);
	if (sub == 0x08)		/* FTS */
		return (range == 0 ? 5000 * vset :
		    range == 1 ? 10000 * vset : -1);
	return (-1);
}

static void
report(const char *when)
{
	uint8_t type = 0, sub = 0, range = 0, vset = 0, en = 0;

	status("%s: reading vdd_gfx", when);
	msm8996_spmi_read(S2_SID, S2_BASE + REG_TYPE, &type);
	msm8996_spmi_read(S2_SID, S2_BASE + REG_SUBTYPE, &sub);
	msm8996_spmi_read(S2_SID, S2_BASE + REG_RANGE, &range);
	msm8996_spmi_read(S2_SID, S2_BASE + REG_VSET, &vset);
	msm8996_spmi_read(S2_SID, S2_BASE + REG_EN, &en);
	status("%s: vdd_gfx %02x/%02x en %02x range %d vset %d; reading MMCC",
	    when, type, sub, en, range, vset);
	status("%s: vdd_gfx type %02x/%02x %s %duV (owner ee%d); "
	    "gdsc gpu 0x%08x/0x%08x gx 0x%08x clamp 0x%x", when, type, sub,
	    (en & EN_BIT) ? "on" : "off", s2_uv(sub, range & 3, vset),
	    msm8996_spmi_owner(S2_SID, S2_BASE), rd(GPU_GDSCR),
	    rd(GPU_GDS_HW_CTRL), rd(GPU_GX_GDSCR), rd(GPU_GX_CLAMP_IO));
	status("%s: clocks mmss_bimc_gfx 0x%08x bimc_gfx 0x%08x "
	    "ahb 0x%08x gfx3d 0x%08x (rcg 0x%x/0x%x) rbbmtimer 0x%08x",
	    when, rd(GCC_MMSS_BIMC_GFX), rd(GCC_BIMC_GFX), rd(GPU_AHB_CBCR),
	    rd(GPU_GX_GFX3D_CBCR), rd(GFX3D_RCG), rd(GFX3D_RCG + 4),
	    rd(GPU_RBBMTIMER_CBCR));
}

static int
power_up(void)
{
	uint8_t sub = 0, range, vset, en;
	int uv, error;

	/* 1. the rail */
	if (msm8996_spmi_read(S2_SID, S2_BASE + REG_SUBTYPE, &sub) ||
	    msm8996_spmi_read(S2_SID, S2_BASE + REG_EN, &en) ||
	    msm8996_spmi_read(S2_SID, S2_BASE + REG_RANGE, &range) ||
	    msm8996_spmi_read(S2_SID, S2_BASE + REG_VSET, &vset)) {
		status("cannot read vdd_gfx");
		return (EIO);
	}
	if ((en & EN_BIT) == 0) {
		uv = s2_uv(sub, range & 3, vset);
		if (uv < 600000 || uv > 1015000) {
			status("vdd_gfx is off and set to %duV; "
			    "not turning it on", uv);
			return (EINVAL);
		}
		error = msm8996_spmi_update(S2_SID, S2_BASE + REG_EN, EN_BIT,
		    EN_BIT);
		if (error) {
			status("vdd_gfx enable: error %d", error);
			return (error);
		}
		DELAY(500);		/* regulator-enable-ramp-delay */
		status("vdd_gfx on at %duV", uv);
	}

	/* 2. the bus clocks and the register interface */
	if ((error = clk_on("mmss_bimc_gfx", GCC_MMSS_BIMC_GFX)) != 0 ||
	    (error = clk_on("bimc_gfx", GCC_BIMC_GFX)) != 0 ||
	    (error = clk_on("gpu_ahb", GPU_AHB_CBCR)) != 0)
		return (error);

	/* 3. the GPU's always-on domain (its status is in hw_ctrl) */
	wr(GPU_GDSCR, rd(GPU_GDSCR) & ~GDSC_SW_COLLAPSE);
	if (waitbits(GPU_GDS_HW_CTRL, GDSC_PWR_ON, GDSC_PWR_ON, 1500) &&
	    waitbits(GPU_GDSCR, GDSC_PWR_ON, GDSC_PWR_ON, 1500)) {
		status("gpu gdsc does not power up (0x%08x 0x%08x)",
		    rd(GPU_GDSCR), rd(GPU_GDS_HW_CTRL));
		return (EIO);
	}

	/* 4. the GX domain, then release its I/O clamp */
	wr(GPU_GX_GDSCR, rd(GPU_GX_GDSCR) & ~GDSC_SW_COLLAPSE);
	if (waitbits(GPU_GX_GDSCR, GDSC_PWR_ON, GDSC_PWR_ON, 1500)) {
		status("gx gdsc does not power up (0x%08x)",
		    rd(GPU_GX_GDSCR));
		return (EIO);
	}
	wr(GPU_GX_CLAMP_IO, rd(GPU_GX_CLAMP_IO) & ~1U);
	DELAY(10);

	/* 5. the core clocks, at whatever rate their roots have now */
	if ((error = clk_on("gfx3d", GPU_GX_GFX3D_CBCR)) != 0 ||
	    (error = clk_on("rbbmtimer", GPU_RBBMTIMER_CBCR)) != 0)
		return (error);
	return (0);
}

static void
gpu_thread(void *arg)
{
	int mode = (int)(intptr_t)arg;

	report("as found");
	if (mode < 2)
		goto out;
	if (power_up() != 0) {
		status("power-up stopped");
		goto out;
	}
	report("powered");
	status("reading GPU registers");
	status("RBBM_STATUS 0x%08x STATUS3 0x%08x INT_0 0x%08x "
	    "CP_PROTECT 0x%08x SECVID_TRUST 0x%08x",
	    rd(GPU_REG(A5XX_RBBM_STATUS)), rd(GPU_REG(A5XX_RBBM_STATUS3)),
	    rd(GPU_REG(A5XX_RBBM_INT_0_STATUS)),
	    rd(GPU_REG(A5XX_CP_PROTECT_STATUS)),
	    rd(GPU_REG(A5XX_RBBM_SECVID_TRUST)));
	status("reading SMMU");
	status("SMMU sCR0 0x%08x IDR0 0x%08x IDR1 0x%08x",
	    rd(SMMU_BASE + SMMU_SCR0), rd(SMMU_BASE + SMMU_IDR0),
	    rd(SMMU_BASE + SMMU_IDR1));
	status("Adreno 530 powered and answering");
out:
	kthread_exit();
}

static void
msm8996_gpu_init(void *dummy __unused)
{
	struct thread *td;
	int mode = 2;

	if (!msm8996_is_present())
		return;
	TUNABLE_INT_FETCH("msm8996.gpu", &mode);
	if (mode <= 0)
		return;
	kthread_create(gpu_thread, (void *)(intptr_t)mode, &td, "gpu");
}
SYSINIT(msm8996gpu, SI_SUB_CONFIGURE + 1, SI_ORDER_ANY, msm8996_gpu_init,
    NULL);
