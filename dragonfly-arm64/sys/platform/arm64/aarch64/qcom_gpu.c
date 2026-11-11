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
#define	MMAGIC_AHB_CBCR		MM(0x5024)
#define	GPU_GX_BCR		MM(0x4020)
#define	GPU_BCR			MM(0x4030)
#define	GPU_AON_BCR		MM(0x4040)
#define	MMAGIC_BIMC_BCR		MM(0x5290)

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

	/*
	 * 2. the GPU's always-on domain (its status is in hw_ctrl).  As in
	 * Linux, the power domains come before the clocks: the GPU's bus
	 * clocks do not run while the domain is collapsed.
	 */
	wr(GPU_GDSCR, rd(GPU_GDSCR) & ~GDSC_SW_COLLAPSE);
	if (waitbits(GPU_GDS_HW_CTRL, GDSC_PWR_ON, GDSC_PWR_ON, 1500) &&
	    waitbits(GPU_GDSCR, GDSC_PWR_ON, GDSC_PWR_ON, 1500)) {
		status("gpu gdsc does not power up (0x%08x 0x%08x)",
		    rd(GPU_GDSCR), rd(GPU_GDS_HW_CTRL));
		return (EIO);
	}
	status("gpu gdsc on (0x%08x 0x%08x)", rd(GPU_GDSCR),
	    rd(GPU_GDS_HW_CTRL));

	/*
	 * 3. the GX domain, as Linux's gdsc_enable() does it for a CLAMP_IO
	 * domain: the wait timings gdsc_init() sets, the I/O clamp released
	 * *before* the collapse bit is cleared, then memories kept on in
	 * the core clock (its cxcs).
	 */
	wr(GPU_GX_GDSCR, (rd(GPU_GX_GDSCR) & ~0x00fff000U) |
	    (0x2 << 20) | (0x8 << 16) | (0x2 << 12));
	/*
	 * The Mi 5's own kernel marks this domain force-enable-root-clk:
	 * the core clock's root (gfx3d_clk_src) and the core clock itself
	 * are turned on before the domain powers up.  ROOT_EN (CMD bit 1)
	 * forces the root on at its current source, XO at reset.
	 */
	wr(GFX3D_RCG, rd(GFX3D_RCG) | 2);
	(void)waitbits(GFX3D_RCG, 1U << 31, 0, 500);
	wr(GPU_GX_GFX3D_CBCR, rd(GPU_GX_GFX3D_CBCR) | CBCR_EN);
	status("gfx3d root 0x%08x/0x%08x, branch 0x%08x", rd(GFX3D_RCG),
	    rd(GFX3D_RCG + 4), rd(GPU_GX_GFX3D_CBCR));
	wr(GPU_GX_CLAMP_IO, rd(GPU_GX_CLAMP_IO) & ~1U);
	DELAY(1);
	wr(GPU_GX_GDSCR, rd(GPU_GX_GDSCR) & ~GDSC_SW_COLLAPSE);
	if (waitbits(GPU_GX_GDSCR, GDSC_PWR_ON, GDSC_PWR_ON, 10000)) {
		uint8_t st1 = 0, st2 = 0, en = 0;

		msm8996_spmi_read(S2_SID, S2_BASE + 0x08, &st1);
		msm8996_spmi_read(S2_SID, S2_BASE + 0x09, &st2);
		msm8996_spmi_read(S2_SID, S2_BASE + REG_EN, &en);
		status("gx gdsc does not power up (0x%08x, clamp 0x%x); "
		    "vdd_gfx status 0x%02x 0x%02x en 0x%02x", rd(GPU_GX_GDSCR),
		    rd(GPU_GX_CLAMP_IO), st1, st2, en);
		return (EIO);
	}
	wr(GPU_GX_GFX3D_CBCR, rd(GPU_GX_GFX3D_CBCR) | (1U << 14) | (1U << 13));
	DELAY(1);
	status("gx gdsc on (0x%08x), clamp 0x%x", rd(GPU_GX_GDSCR),
	    rd(GPU_GX_CLAMP_IO));

	/*
	 * 4a. the multimedia AHB interface clock, which the Mi 5's own GPU
	 * driver keeps on (alt_mem_iface_clk = mmss_mmagic_ahb_clk); its
	 * status is the enable bit (check_enable_bit downstream).
	 */
	wr(MMAGIC_AHB_CBCR, rd(MMAGIC_AHB_CBCR) | CBCR_EN);
	DELAY(10);

	/* 4b. block resets: any still asserted (bit 0) is released */
	status("resets: gpu_gx 0x%x gpu 0x%x gpu_aon 0x%x mmagic_bimc 0x%x; "
	    "mmagic_ahb 0x%08x", rd(GPU_GX_BCR), rd(GPU_BCR), rd(GPU_AON_BCR),
	    rd(MMAGIC_BIMC_BCR), rd(MMAGIC_AHB_CBCR));
	{
		static const vm_paddr_t bcr[] = { GPU_AON_BCR, GPU_BCR,
		    GPU_GX_BCR, MMAGIC_BIMC_BCR };

		for (int i = 0; i < 4; ++i)
			if (rd(bcr[i]) & 1)
				wr(bcr[i], rd(bcr[i]) & ~1U);
		DELAY(10);
	}

	/*
	 * 4. the clocks, each tried and reported; the GPU's registers are
	 * only read if the register interface (ahb) and core (gfx3d) run.
	 */
	{
		int e1 = clk_on("mmss_bimc_gfx", GCC_MMSS_BIMC_GFX);
		int e2 = clk_on("bimc_gfx", GCC_BIMC_GFX);
		int e3 = clk_on("gpu_ahb", GPU_AHB_CBCR);
		int e4 = clk_on("gfx3d", GPU_GX_GFX3D_CBCR);
		int e5 = clk_on("rbbmtimer", GPU_RBBMTIMER_CBCR);

		status("clocks: mmss_bimc_gfx %s, bimc_gfx %s, ahb %s, "
		    "gfx3d %s, rbbmtimer %s", e1 ? "off" : "on",
		    e2 ? "off" : "on", e3 ? "off" : "on", e4 ? "off" : "on",
		    e5 ? "off" : "on");
		if (e3 || e4)
			return (EIO);
	}
	return (0);
}

static int gpu_powered;

/*
 * sysctl hw.mi5.gpu_read=1: read GPU and SMMU registers, one at a time
 * with a status line before each, so a read that stalls the bus names
 * itself (and only the reading process hangs, not the boot).
 */
static int
sysctl_gpu_read(SYSCTL_HANDLER_ARGS)
{
	static const struct { vm_paddr_t pa; const char *n; } regs[] = {
		{ GPU_REG(A5XX_RBBM_STATUS), "RBBM_STATUS" },
		{ GPU_REG(A5XX_RBBM_STATUS3), "RBBM_STATUS3" },
		{ GPU_REG(A5XX_RBBM_INT_0_STATUS), "RBBM_INT_0_STATUS" },
		{ GPU_REG(A5XX_CP_PROTECT_STATUS), "CP_PROTECT_STATUS" },
		{ GPU_REG(A5XX_RBBM_SECVID_TRUST), "RBBM_SECVID_TRUST" },
		{ SMMU_BASE + SMMU_SCR0, "SMMU sCR0" },
		{ SMMU_BASE + SMMU_IDR0, "SMMU IDR0" },
		{ SMMU_BASE + SMMU_IDR1, "SMMU IDR1" },
	};
	int v = 0, error, i;

	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	if (!gpu_powered)
		return (ENXIO);
	for (i = 0; i < (int)NELEM(regs); ++i) {
		uint32_t val;

		status("reading %s at 0x%jx", regs[i].n, (uintmax_t)regs[i].pa);
		uprintf("gpu: reading %s at 0x%jx ... ", regs[i].n,
		    (uintmax_t)regs[i].pa);
		val = rd(regs[i].pa);
		uprintf("0x%08x\n", val);
		status("%s = 0x%08x", regs[i].n, val);
	}
	status("Adreno 530 powered and answering");
	return (0);
}
SYSCTL_PROC(_hw_mi5, OID_AUTO, gpu_read, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_gpu_read, "I", "read GPU registers (may hang the caller)");

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
	gpu_powered = 1;
	status("powered; sysctl hw.mi5.gpu_read=1 reads GPU registers");
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
