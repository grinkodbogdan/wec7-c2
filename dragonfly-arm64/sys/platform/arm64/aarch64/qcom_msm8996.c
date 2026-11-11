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
 * Board support for phones built on the Qualcomm MSM8996 (Snapdragon
 * 820/821), written for the Xiaomi Mi 5 (gemini).
 *
 * This is the part of bring-up that no device tree property expresses:
 * the state the Little Kernel (LK) bootloader leaves the hardware in.
 * The facts come from the working Mi 5 port in the rpiios project
 * (mi5/README.md and its loader shim), which found them on a real
 * phone:
 *
 *   - LK leaves the APSS watchdog running.  Unless it is stopped the
 *     phone resets a few seconds into boot.
 *   - LK leaves the display on, showing its splash screen from a
 *     framebuffer whose address, stride and depth must be read back
 *     from the display controller (MDP): they differ between panels.
 *   - The Mi 5's panels are command-mode ones.  They do not refresh by
 *     themselves; each frame has to be sent with a "start" request.
 *   - The secure firmware forbids access to the GICv3 redistributor's
 *     GICR_WAKER register; touching it hangs the phone.  Linux has the
 *     same workaround (FLAGS_WORKAROUND_GICR_WAKER_MSM8996).  The
 *     vendor device trees do not carry the "qcom,msm8996-gic-v3"
 *     compatible that Linux keys it on, so it is set here by SoC.
 *   - Android's "ramoops" console zone at 0x92920000 survives a warm
 *     reset, so text written there can be read back afterwards from a
 *     recovery image.  That is the only log there is when the screen
 *     shows nothing.
 *
 * Everything here is reached only when the device tree's root node is
 * compatible with "qcom,msm8996".
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

/* APSS watchdog (device tree: qcom,msm-watchdog) */
#define	WDT_EN			0x08

/* Mobile Display Processor.  Offsets from the MDSS block at 0x900000. */
#define	MDP_BASE		0x901000UL
#define	MDP_INTR_STATUS		(MDP_BASE + 0x14)
#define	MDP_INTR_CLEAR		(MDP_BASE + 0x18)
#define	MDP_INTR_PP0_DONE	(1U << 8)	/* PING_PONG_0_DONE */
#define	MDP_CTL0_LAYER0		0x902000UL	/* pipes staged on mixer 0 */
#define	MDP_CTL0_OP		0x902014UL
#define	MDP_CTL0_OP_CMD_MODE	(1U << 17)
#define	MDP_CTL0_FLUSH		0x902018UL
#define	  CTL_FLUSH_LM0		(1U << 6)
#define	  CTL_FLUSH_CTL		(1U << 17)
#define	MDP_CTL0_START		0x90201cUL

/* Source pipe (SSPP) registers */
#define	SSPP_SRC_SIZE		0x00		/* height << 16 | width */
#define	SSPP_SRC0_ADDR		0x14
#define	SSPP_SRC_YSTRIDE	0x24
#define	SSPP_SRC_FORMAT		0x30		/* [10:9] = bytes/pixel - 1 */

/* Where LK's splash lives unless the MDP says otherwise. */
#define	MI5_SPLASH_FB		0x83401000UL
#define	MI5_WIDTH		1080
#define	MI5_HEIGHT		1920

/* ramoops console zone, in Linux's persistent_ram_buffer format */
#define	MI5_RAMOOPS_CONSOLE	0x92920000UL
#define	PRZ_SIZE_MAX		(0x80000UL - 12)
#define	PRZ_SIG			0x43474244U	/* "DBGC" */

struct prz_buffer {
	uint32_t	sig;
	uint32_t	start;		/* write position */
	uint32_t	size;		/* bytes valid */
	uint8_t		data[];
};

static int msm8996_cmd_mode;
static int msm8996_present;
static vm_paddr_t msm8996_pipe;		/* source pipe scanning out the fb */
static uint32_t msm8996_pipe_flush;
static volatile struct prz_buffer *ramlog;

static __inline uint32_t
rd32(vm_paddr_t pa)
{
	return (*(volatile uint32_t *)PHYS_TO_DMAP(pa));
}

static __inline void
wr32(vm_paddr_t pa, uint32_t val)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(pa) = val;
}

/*
 * Find the framebuffer LK is scanning out: the first source pipe that
 * is staged on layer mixer 0 and has a buffer address.
 */
static void
msm8996_probe_display(struct arm64_bootinfo *bi)
{
	static const struct {
		u_int		shift;	/* field in CTL0_LAYER0 */
		vm_paddr_t	pipe;
		uint32_t	flush;	/* its bit in CTL0_FLUSH */
	} stages[] = {
		{ 0,  MDP_BASE + 0x4000,  1U << 0 },	/* VIG0 */
		{ 3,  MDP_BASE + 0x6000,  1U << 1 },	/* VIG1 */
		{ 9,  MDP_BASE + 0x14000, 1U << 3 },	/* RGB0 */
		{ 12, MDP_BASE + 0x16000, 1U << 4 },	/* RGB1 */
		{ 18, MDP_BASE + 0x24000, 1U << 11 },	/* DMA0 */
	};
	uint32_t layer = rd32(MDP_CTL0_LAYER0);
	uint32_t addr, stride, bytes, size;
	vm_paddr_t pipe = 0;
	u_int i;

	msm8996_cmd_mode = (rd32(MDP_CTL0_OP) & MDP_CTL0_OP_CMD_MODE) != 0;

	for (i = 0; i < NELEM(stages); ++i) {
		if (((layer >> stages[i].shift) & 7) != 0 &&
		    rd32(stages[i].pipe + SSPP_SRC0_ADDR) != 0) {
			pipe = stages[i].pipe;
			msm8996_pipe = pipe;
			msm8996_pipe_flush = stages[i].flush;
			break;
		}
	}

	/* Defaults: what LK is known to set up on this phone. */
	bi->bi_fb_base = MI5_SPLASH_FB;
	bi->bi_fb_width = MI5_WIDTH;
	bi->bi_fb_height = MI5_HEIGHT;
	bi->bi_fb_bpp = 24;
	bi->bi_fb_stride = MI5_WIDTH * 3;
	if (pipe == 0)
		return;

	addr = rd32(pipe + SSPP_SRC0_ADDR);
	stride = rd32(pipe + SSPP_SRC_YSTRIDE) & 0xffff;
	bytes = ((rd32(pipe + SSPP_SRC_FORMAT) >> 9) & 3) + 1;
	size = rd32(pipe + SSPP_SRC_SIZE);
	if ((bytes != 3 && bytes != 4) || stride < MI5_WIDTH * bytes)
		return;			/* not something we can draw on */
	bi->bi_fb_base = addr;
	bi->bi_fb_stride = stride;
	bi->bi_fb_bpp = bytes * 8;
	if ((size & 0xffff) >= 320 && (size >> 16) >= 320 &&
	    (size & 0xffff) * bytes <= stride) {
		bi->bi_fb_width = size & 0xffff;
		bi->bi_fb_height = size >> 16;
	}
}

/*
 * Send the framebuffer to a command-mode panel.  A transfer takes up
 * to about 32 ms (it waits for the panel's tearing signal), and a new
 * one must not be started while one is in flight, so wait for the
 * "done" interrupt status, bounded in case it never comes.
 */
void
msm8996_fb_flush(void)
{
	uint64_t freq, t0;

	if (!msm8996_cmd_mode)
		return;
	wr32(MDP_INTR_CLEAR, MDP_INTR_PP0_DONE);
	cpu_mfence();
	wr32(MDP_CTL0_START, 1);

	freq = arm64_timer_freq();
	t0 = cpu_cntvct();
	while ((rd32(MDP_INTR_STATUS) & MDP_INTR_PP0_DONE) == 0 &&
	    cpu_cntvct() - t0 < freq / 10)
		;
}

/*
 * Append a character to the RAM log.
 */
void
arm64_ramlog_putc(int c)
{
	volatile struct prz_buffer *prz = ramlog;
	uint32_t pos;

	if (prz == NULL)
		return;
	pos = prz->start;
	if (pos >= PRZ_SIZE_MAX)
		return;			/* full: keep the start of the boot */
	prz->data[pos++] = (uint8_t)c;
	prz->start = pos;
	prz->size = pos;
}

static void
ramlog_init(vm_paddr_t pa)
{
	volatile struct prz_buffer *prz;

	prz = (volatile struct prz_buffer *)PHYS_TO_DMAP(pa);
	prz->start = 0;
	prz->size = 0;
	prz->sig = PRZ_SIG;
	ramlog = prz;
}

static int
overlaps(vm_paddr_t a, vm_size_t alen, vm_paddr_t b, vm_size_t blen)
{
	return (a < b + blen && b < a + alen);
}

/*
 * LK puts the boot image's ramdisk at 0x82a00000 and keeps its splash
 * screen at 0x83401000, so a ramdisk over about 10MB ends inside the
 * framebuffer, and every line the console draws overwrites part of it.
 * Give the display a buffer of its own after the ramdisk and point the
 * scanout pipe at it.  initarm() reserves whatever bi_fb_base is.
 */
static void
msm8996_move_fb(struct arm64_bootinfo *bi)
{
	vm_size_t size = (vm_size_t)bi->bi_fb_stride * bi->bi_fb_height;
	vm_paddr_t new;
	int i, moved;

	if (bi->bi_initrd_end <= bi->bi_initrd_start || msm8996_pipe == 0 ||
	    !overlaps(bi->bi_fb_base, size, bi->bi_initrd_start,
	    bi->bi_initrd_end - bi->bi_initrd_start))
		return;

	new = roundup2(bi->bi_initrd_end, 0x100000);
	do {			/* step past no-map reserved regions */
		moved = 0;
		for (i = 0; i < bi->bi_nresv; ++i) {
			if (bi->bi_resv_nomap[i] &&
			    overlaps(new, size, bi->bi_resv[i].mr_start,
			    bi->bi_resv[i].mr_size)) {
				new = roundup2(bi->bi_resv[i].mr_start +
				    bi->bi_resv[i].mr_size, 0x100000);
				moved = 1;
			}
		}
	} while (moved);
	for (i = 0; i < bi->bi_nmem; ++i) {
		if (new >= bi->bi_mem[i].mr_start && new + size <=
		    bi->bi_mem[i].mr_start + bi->bi_mem[i].mr_size)
			break;
	}
	if (i == bi->bi_nmem)
		return;			/* nowhere to go; keep LK's */

	bzero((void *)PHYS_TO_DMAP(new), size);
	__asm __volatile("dsb sy");
	{
		vm_offset_t p, e = PHYS_TO_DMAP(new) + size;

		for (p = PHYS_TO_DMAP(new); p < e; p += 64)
			__asm __volatile("dc cvac, %0" : : "r" (p));
		__asm __volatile("dsb sy");
	}
	wr32(msm8996_pipe + SSPP_SRC0_ADDR, (uint32_t)new);
	wr32(MDP_CTL0_FLUSH, msm8996_pipe_flush | CTL_FLUSH_LM0 |
	    CTL_FLUSH_CTL);
	cpu_mfence();
	bi->bi_fb_moved_from = bi->bi_fb_base;
	bi->bi_fb_base = new;
}

/*
 * Called from initarm() once the device tree has been scanned, before
 * the console is set up.  Returns non-zero if this is an MSM8996.
 */
int
msm8996_early_init(struct arm64_bootinfo *bi)
{
	const char *p = bi->bi_compatible;
	const char *end = p + bi->bi_compatible_len;
	int match = 0;

	while (p != NULL && p < end) {
		if (strcmp(p, "qcom,msm8996") == 0 ||
		    strcmp(p, "qcom,msm8996pro") == 0)
			match = 1;
		p += strlen(p) + 1;
	}
	if (!match)
		return (0);

	msm8996_present = 1;
	bi->bi_gic_no_waker = 1;

	/* First of all: the watchdog, before it bites. */
	if (bi->bi_wdt_base != 0) {
		wr32(bi->bi_wdt_base + WDT_EN, 0);
		cpu_mfence();
	}

	ramlog_init(MI5_RAMOOPS_CONSOLE);

	/* An fb= boot argument overrides what the MDP reports. */
	if (bi->bi_fb_base == 0) {
		msm8996_probe_display(bi);
		msm8996_move_fb(bi);
	}
	return (1);
}

/*
 * Kryo cpu clocks (APCC, "qcom,msm8996-apcc" at 0x6400000).
 *
 * The bootloader clocks the power cluster (cpus 0-1) it runs on and
 * leaves the performance cluster (cpus 2-3, mpidr 0x100/0x101) on the
 * 19.2 MHz XO.  Linux's clk-cpu-8996 driver reprograms both clusters
 * at probe, before cpufreq touches voltages: muxes to GPLL0 (300 MHz)
 * while the cluster PLL is configured for L=54 (1036.8 MHz), then the
 * muxes to the PLL.  Do the same for the performance cluster only,
 * leaving the bootloader's power cluster setting alone.  Linux takes
 * the ACD leg (mux 0x32), which needs the cluster's L2 ACD set up from
 * a cpu in that cluster; the plain PLL leg (mux 0x31) needs nothing.
 *
 * Boot argument msm8996.perfcl: 0 leaves the cluster alone, 1 stops at
 * GPLL0 (300 MHz), 2 (default) runs it from its PLL.
 */
#define	APCC_BASE		0x06400000UL
#define	PERFCL			(APCC_BASE + 0x80000)
#define	APC_PLL_MODE		0x00
#define	  PLL_FSM_ENA		(1U << 20)
#define	  PLL_FSM_RESET		(1U << 21)
#define	  PLL_ACTIVE		(1U << 30)
#define	  PLL_LOCK_DET		(1U << 31)
#define	APC_PLL_L		0x04
#define	APC_PLL_ALPHA		0x08
#define	APC_PLL_USER_CTL	0x10
#define	APC_PLL_CONFIG		0x18
#define	APC_PLL_CONFIG_U	0x1c
#define	APC_PLL_TEST		0x20
#define	APC_PLL_TEST_U		0x24
#define	APC_MUX			0x40
#define	  MUX_AUTO_SEL_GPLL0	(3U << 4)
#define	APC_CLK_CTL		0x44
#define	  CLK_CTL_AUTO_SEL	(1U << 8)
#define	APC_PSCTL		0x164

static void
apcc_rmw(vm_paddr_t reg, uint32_t mask, uint32_t val)
{
	wr32(reg, (rd32(reg) & ~mask) | val);
}

int
msm8996_cpu_clocks(void)
{
	vm_paddr_t b = PERFCL;
	int mode = 2, i;
	uint32_t m;

	if (!msm8996_present)
		return (0);
	TUNABLE_INT_FETCH("msm8996.perfcl", &mode);
	kprintf("msm8996: performance cluster clock mux %#x, pll mode %#x\n",
	    rd32(b + APC_MUX), rd32(b + APC_PLL_MODE));
	if (mode <= 0)
		return (0);

	/* Run from GPLL0 (300 MHz) while the PLL is set up. */
	wr32(b + APC_MUX, 0xc);
	cpu_mfence();
	DELAY(5);
	apcc_rmw(b + APC_MUX, 3U << 4, MUX_AUTO_SEL_GPLL0);
	if (mode == 1) {
		kprintf("msm8996: performance cluster on GPLL0\n");
		return (1);
	}

	/* clk_alpha_pll_configure() with Linux's hfpll_config */
	wr32(b + APC_PLL_L, 54);
	wr32(b + APC_PLL_ALPHA, 0);
	wr32(b + APC_PLL_CONFIG, 0x200d4828);
	wr32(b + APC_PLL_CONFIG_U, 0x006);
	apcc_rmw(b + APC_PLL_USER_CTL, (1U << 0) | (1U << 3) | (1U << 12) |
	    (3U << 8), (1U << 0) | (1U << 3) | (1U << 8));
	wr32(b + APC_PLL_TEST, 0x1c000000);
	wr32(b + APC_PLL_TEST_U, 0x00004000);
	/* qcom_pll_set_fsm_mode(6, 0): the hardware sequences the PLL */
	apcc_rmw(b + APC_PLL_MODE, PLL_FSM_RESET, 0);
	apcc_rmw(b + APC_PLL_MODE, (0x3fU << 14) | (0x3fU << 8), 6U << 14);
	apcc_rmw(b + APC_PLL_MODE, PLL_FSM_ENA, PLL_FSM_ENA);
	cpu_mfence();

	for (i = 0; i < 1000; ++i) {
		m = rd32(b + APC_PLL_MODE);
		if ((m & PLL_ACTIVE) && (m & PLL_LOCK_DET))
			break;
		DELAY(1);
	}
	DELAY(50);
	if ((m & PLL_LOCK_DET) == 0) {
		kprintf("msm8996: performance cluster PLL did not lock "
		    "(mode %#x); staying on GPLL0\n", m);
		return (1);
	}

	apcc_rmw(b + APC_CLK_CTL, CLK_CTL_AUTO_SEL, CLK_CTL_AUTO_SEL);
	cpu_mfence();
	DELAY(5);
	wr32(b + APC_PSCTL, 0x00030005);	/* pulse swallower, soft start */
	wr32(b + APC_MUX, 0x31);		/* the PLL leg */
	cpu_mfence();
	kprintf("msm8996: performance cluster on its PLL (1036.8 MHz), "
	    "mode %#x\n", rd32(b + APC_PLL_MODE));
	return (1);
}
