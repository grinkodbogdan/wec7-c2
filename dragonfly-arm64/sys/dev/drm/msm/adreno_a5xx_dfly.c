// SPDX-License-Identifier: GPL-2.0-only
/*
 * Adreno 5xx bring-up for DragonFly/arm64 (the Xiaomi Mi 5's Adreno 530).
 *
 * Derived from Linux's drivers/gpu/drm/msm/adreno/a5xx_gpu.c (a5xx_hw_init,
 * a5xx_me_init, a5xx_flush, a5xx_idle) and adreno_gpu.c / adreno_gpu.h
 * (firmware buffers, PM4 packets):
 *   Copyright (c) 2016-2017 The Linux Foundation. All rights reserved.
 * DragonFly adaptation Copyright (c) 2026 The DragonFly Project.
 *
 * This file is GPL-2.0, kept apart from the BSD-licensed platform code as
 * the project's README describes.  This is a first, cut-down step: the
 * ring, the microcode, the register set-up of a5xx_hw_init, CP_ME_INIT,
 * the secure-mode exit through the zap shader, and one CP_MEM_WRITE to
 * show that the GPU executes commands.  Left out for now: hardware clock
 * gating, the GPMU (power management), preemption, interrupts and the
 * GPU's own page tables (its stream goes through the SMMU in bypass).
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/malloc.h>

#include <vm/vm.h>
#include <vm/pmap.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#include "adreno_a5xx_dfly.h"

#define	GPU_BASE	0x00b00000UL
#define	SMMU_BASE	0x00b40000UL

/* registers (dword offsets, from Mesa's a5xx.xml / adreno_common.xml) */
#define	REG_A5XX_VBIF_ROUND_ROBIN_QOS_ARB	0x3049
#define	REG_A5XX_RBBM_PERFCTR_GPU_BUSY_MASKED	0x046f
#define	REG_A5XX_RBBM_AHB_CNTL0			0x0093
#define	REG_A5XX_RBBM_AHB_CNTL1			0x0094
#define	REG_A5XX_RBBM_AHB_CNTL2			0x0095
#define	REG_A5XX_RBBM_INTERFACE_HANG_MASK_CNTL11 0x009c
#define	REG_A5XX_RBBM_INTERFACE_HANG_INT_CNTL	0x002f
#define	REG_A5XX_RBBM_PERFCTR_CNTL		0x0464
#define	REG_A5XX_CP_PERFCTR_CP_SEL_0		0x0bb0
#define	REG_A5XX_RBBM_PERFCTR_RBBM_SEL_0	0x046b
#define	REG_A5XX_UCHE_CACHE_WAYS		0x0e96
#define	REG_A5XX_UCHE_TRAP_BASE_LO		0x0e89
#define	REG_A5XX_UCHE_TRAP_BASE_HI		0x0e8a
#define	REG_A5XX_UCHE_WRITE_THRU_BASE_LO	0x0e87
#define	REG_A5XX_UCHE_WRITE_THRU_BASE_HI	0x0e88
#define	REG_A5XX_UCHE_GMEM_RANGE_MIN_LO		0x0e8b
#define	REG_A5XX_UCHE_GMEM_RANGE_MIN_HI		0x0e8c
#define	REG_A5XX_UCHE_GMEM_RANGE_MAX_LO		0x0e8d
#define	REG_A5XX_UCHE_GMEM_RANGE_MAX_HI		0x0e8e
#define	REG_A5XX_CP_MEQ_THRESHOLDS		0x0825
#define	REG_A5XX_CP_MERCIU_SIZE			0x0826
#define	REG_A5XX_CP_ROQ_THRESHOLDS_2		0x0820
#define	REG_A5XX_CP_ROQ_THRESHOLDS_1		0x081f
#define	REG_A5XX_PC_DBG_ECO_CNTL		0x0d00
#define	REG_A5XX_UCHE_MODE_CNTL			0x0e81
#define	REG_A5XX_CP_CHICKEN_DBG			0x0833
#define	REG_A5XX_TPL1_MODE_CNTL			0x0f02
#define	REG_A5XX_RB_MODE_CNTL			0x0cc6
#define	REG_A5XX_VPC_DBG_ECO_CNTL		0x0e60
#define	REG_A5XX_CP_PROTECT_CNTL		0x08a0
#define	REG_A5XX_CP_PROTECT(n)			(0x0880 + (n))
#define	REG_A5XX_RBBM_SECVID_TSB_TRUSTED_BASE_LO 0xf800
#define	REG_A5XX_RBBM_SECVID_TSB_TRUSTED_BASE_HI 0xf801
#define	REG_A5XX_RBBM_SECVID_TSB_TRUSTED_SIZE	0xf802
#define	REG_A5XX_RBBM_SECVID_TSB_CNTL		0xf803
#define	REG_A5XX_RBBM_SECVID_TRUST_CNTL		0xf400
#define	REG_A5XX_CP_ADDR_MODE_CNTL		0x0860
#define	REG_A5XX_VSC_ADDR_MODE_CNTL		0x0bc1
#define	REG_A5XX_GRAS_ADDR_MODE_CNTL		0x0c81
#define	REG_A5XX_RB_ADDR_MODE_CNTL		0x0cc5
#define	REG_A5XX_PC_ADDR_MODE_CNTL		0x0d01
#define	REG_A5XX_HLSQ_ADDR_MODE_CNTL		0x0e05
#define	REG_A5XX_VFD_ADDR_MODE_CNTL		0x0e41
#define	REG_A5XX_VPC_ADDR_MODE_CNTL		0x0e61
#define	REG_A5XX_UCHE_ADDR_MODE_CNTL		0x0e80
#define	REG_A5XX_SP_ADDR_MODE_CNTL		0x0ec1
#define	REG_A5XX_TPL1_ADDR_MODE_CNTL		0x0f01
#define	REG_A5XX_RBBM_SECVID_TSB_ADDR_MODE_CNTL	0xf810
#define	REG_A5XX_CP_ME_INSTR_BASE_LO		0x0838
#define	REG_A5XX_CP_ME_INSTR_BASE_HI		0x0839
#define	REG_A5XX_CP_PFP_INSTR_BASE_LO		0x0835
#define	REG_A5XX_CP_PFP_INSTR_BASE_HI		0x0836
#define	REG_A5XX_CP_RB_BASE			0x0800
#define	REG_A5XX_CP_RB_BASE_HI			0x0801
#define	REG_A5XX_CP_RB_CNTL			0x0802
#define	REG_A5XX_CP_RB_RPTR_ADDR		0x0804
#define	REG_A5XX_CP_RB_RPTR_ADDR_HI		0x0805
#define	REG_A5XX_CP_RB_RPTR			0x0806
#define	REG_A5XX_CP_RB_WPTR			0x0807
#define	REG_A5XX_RBBM_INT_0_MASK		0x0038
#define	REG_A5XX_CP_PFP_ME_CNTL			0x0832
#define	REG_A5XX_RBBM_STATUS			0x04f5
#define	REG_A5XX_RBBM_SW_RESET_CMD		0x0037
#define	REG_A5XX_GPMU_SP_POWER_CNTL		0xa881
#define	REG_A5XX_GPMU_RBCCU_POWER_CNTL		0xa887
#define	REG_A5XX_GPMU_SP_PWR_CLK_STATUS		0xa88b
#define	REG_A5XX_GPMU_RBCCU_PWR_CLK_STATUS	0xa88d
#define	A5XX_GPMU_PWR_ON			(1U << 20)
#define	REG_A5XX_RBBM_INT_0_STATUS		0x04e1
#define	REG_A5XX_CP_HW_FAULT			0x0b1a
#define	REG_A5XX_CP_INTERRUPT_STATUS		0x0b18
#define	REG_A5XX_CP_PROTECT_STATUS		0x0b1c
#define	REG_A5XX_CP_SCRATCH_REG(n)		(0x0b78 + (n))

#define	PERF_CP_ALWAYS_COUNT	0

/* PM4 (adreno_gpu.h, adreno_pm4.xml) */
#define	CP_TYPE4_PKT		(4U << 28)
#define	CP_TYPE7_PKT		(7U << 28)
#define	CP_WAIT_FOR_IDLE	0x26
#define	CP_MEM_WRITE		0x3d
#define	CP_ME_INIT		0x48
#define	CP_SET_SECURE_MODE	0x66
#define	CP_INDIRECT_BUFFER_PFE	0x3f
#define	CP_EVENT_WRITE		0x46
#define	CP_SET_RENDER_MODE	0x6c
#define	CACHE_FLUSH_TS		4
#define	REG_A5XX_RBBM_ALWAYSON_COUNTER_LO	0x04d2
#define	REG_A5XX_RBBM_ALWAYSON_COUNTER_HI	0x04d3
#define	FENCE_OFFSET		0x100	/* in the scratch page */

#define	RB_SIZE			(32 * 1024)	/* MSM_GPU_RINGBUFFER_SZ */
#define	RB_BLKSIZE		32
#define	RB_CNTL_NO_UPDATE	(1U << 27)

/* the Adreno 530's (adreno_device.c, a5xx_gpu.c) */
#define	A530_GMEM		(1024 * 1024)
#define	A530_HIGHEST_BANK_BIT	15

MALLOC_DEFINE(M_A5XX, "a5xx", "Adreno 5xx");

static struct a5xx_dfly {
	a5xx_status_fn	status;
	uint32_t	*rb;		/* the ring */
	vm_paddr_t	rb_pa;
	uint32_t	wptr;		/* in dwords */
	void		*pm4, *pfp;
	size_t		pm4_len, pfp_len;
	vm_paddr_t	pm4_pa, pfp_pa;
	uint32_t	*scratch;
	vm_paddr_t	scratch_pa;
} a5;

static void smmu_faults(void);
static int a5_running;

static __inline uint32_t
gpu_read(uint32_t reg)
{
	return (*(volatile uint32_t *)PHYS_TO_DMAP(GPU_BASE + 4 * reg));
}

static __inline void
gpu_write(uint32_t reg, uint32_t v)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(GPU_BASE + 4 * reg) = v;
}

static __inline void
gpu_write64(uint32_t reg, uint64_t v)
{
	gpu_write(reg, (uint32_t)v);
	gpu_write(reg + 1, (uint32_t)(v >> 32));
}

static __inline void
gpu_rmw(uint32_t reg, uint32_t mask, uint32_t or)
{
	gpu_write(reg, (gpu_read(reg) & ~mask) | or);
}

static __inline uint32_t
smmu_read(uint32_t off)
{
	return (*(volatile uint32_t *)PHYS_TO_DMAP(SMMU_BASE + off));
}

static __inline void
smmu_write(uint32_t off, uint32_t v)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(SMMU_BASE + off) = v;
}

/* the GPU is not coherent with the cpu's caches */
static void
dcache_wbinv(void *p, size_t len)
{
	vm_offset_t a = (vm_offset_t)p & ~63UL, end = (vm_offset_t)p + len;

	for (; a < end; a += 64)
		__asm __volatile("dc civac, %0" : : "r" (a) : "memory");
	__asm __volatile("dsb sy" : : : "memory");
}

static __inline uint32_t
PM4_PARITY(uint32_t val)
{
	return (0x9669 >> (0xF & (val ^
		(val >> 4) ^ (val >> 8) ^ (val >> 12) ^
		(val >> 16) ^ ((val) >> 20) ^ (val >> 24) ^
		(val >> 28)))) & 1;
}

static __inline void
OUT_RING(uint32_t v)
{
	a5.rb[a5.wptr] = v;
	a5.wptr = (a5.wptr + 1) % (RB_SIZE / 4);
}

static __inline void
OUT_PKT4(uint32_t reg, uint16_t cnt)
{
	OUT_RING(CP_TYPE4_PKT | (cnt << 0) | (PM4_PARITY(cnt) << 7) |
	    ((reg & 0x3FFFF) << 8) | (PM4_PARITY(reg) << 27));
}

static __inline void
OUT_PKT7(uint8_t opcode, uint16_t cnt)
{
	OUT_RING(CP_TYPE7_PKT | (cnt << 0) | (PM4_PARITY(cnt) << 15) |
	    ((opcode & 0x7F) << 16) | (PM4_PARITY(opcode) << 23));
}

/* a5xx_flush: make the ring visible and move the write pointer */
static void
a5xx_flush(void)
{
	dcache_wbinv(a5.rb, RB_SIZE);
	gpu_write(REG_A5XX_CP_RB_WPTR, a5.wptr);
}

/*
 * a5xx_idle, polled: the CP has read everything and the GPU is quiet.
 * Linux waits for RBBM_STATUS without HI_BUSY (bit 0) to read 0, but on
 * this phone, with no GPMU and no hardware clock gating yet, the summary
 * bits GPU_BUSY_NOHC (31), GPU_BUSY (30) and GPU_BUSY_HYST (8) are
 * already set before the CP first runs (0xc0000101) and stay set.  So
 * only the per-block busy bits count: the CP's (1-7) and the pipeline's
 * (9-29).  Whether commands really ran is checked by CP_MEM_WRITE.
 */
#define	A5XX_RBBM_STATUS_BLOCKS	0x3ffffefe

static int
a5xx_idle_n(const char *what, int n)
{
	int i;

	for (i = 0; i < n; ++i) {
		if (gpu_read(REG_A5XX_CP_RB_RPTR) == a5.wptr &&
		    (gpu_read(REG_A5XX_RBBM_STATUS) &
		    A5XX_RBBM_STATUS_BLOCKS) == 0)
			return (0);
		DELAY(10);
	}
	a5.status("a5xx: %s: timeout: status 0x%08x irq 0x%08x rptr/wptr "
	    "%u/%u hw_fault 0x%08x cp_int 0x%08x protect 0x%08x", what,
	    gpu_read(REG_A5XX_RBBM_STATUS), gpu_read(REG_A5XX_RBBM_INT_0_STATUS),
	    gpu_read(REG_A5XX_CP_RB_RPTR), a5.wptr,
	    gpu_read(REG_A5XX_CP_HW_FAULT),
	    gpu_read(REG_A5XX_CP_INTERRUPT_STATUS),
	    gpu_read(REG_A5XX_CP_PROTECT_STATUS));
	smmu_faults();
	return (ETIMEDOUT);
}

static int
a5xx_idle(const char *what)
{
	return (a5xx_idle_n(what, 100000));		/* 1 s */
}

static int
ilog2_u(uint32_t v)
{
	int r = -1;

	while (v) {
		v >>= 1;
		r++;
	}
	return (r);
}

#define	ADRENO_PROTECT_RW(_reg, _len) \
	((1 << 30) | (1 << 29) | \
	((ilog2_u((_len)) & 0x1F) << 24) | (((_reg) << 2) & 0xFFFFF))

/*
 * The GPU's stream through its SMMU, untranslated (physical addresses):
 * on the stream match whose id is 0 (the device tree's iommus =
 * <&smmu 0>), or a free one.  Plain bypass (S2CR type 1) first; where
 * the SMMU refuses it (it reads back as fault), the way Linux's
 * arm-smmu-qcom.c handles that (qcom_smmu_cfg_probe, bypass_quirk): the
 * last context bank, stage 1 with translation off (SCTLR 0) and stage 2
 * bypassed, and the stream translated through it.
 */
static int smmu_cb = -1;
static uint32_t smmu_pgsize, smmu_numpage;

static vm_paddr_t
smmu_cb_base(int n)
{
	return (SMMU_BASE + (vm_paddr_t)(smmu_numpage + n) * smmu_pgsize);
}

static int
smmu_bypass(void)
{
	uint32_t scr0 = smmu_read(0), idr0 = smmu_read(0x20);
	uint32_t idr1 = smmu_read(0x24), s2cr;
	int nsmr = idr0 & 0xff, numcb = idr1 & 0xff, i, use = -1, free = -1;

	smmu_pgsize = (idr1 & (1U << 31)) ? 65536 : 4096;
	smmu_numpage = 1U << (((idr1 >> 28) & 7) + 1);
	for (i = 0; i < nsmr; ++i) {
		uint32_t smr = smmu_read(0x800 + 4 * i);

		s2cr = smmu_read(0xc00 + 4 * i);
		a5.status("a5xx: smmu SMR%d 0x%08x S2CR%d 0x%08x", i, smr, i,
		    s2cr);
		if ((smr & (1U << 31)) && (smr & 0x7fff) == 0 && use < 0)
			use = i;
		if (!(smr & (1U << 31)) && free < 0)
			free = i;
	}
	if (use < 0)
		use = free;
	if (use < 0) {
		a5.status("a5xx: no stream match register for the GPU");
		return (ENXIO);
	}
	smmu_write(0xc00 + 4 * use, 1U << 16);		/* bypass */
	/*
	 * Valid, id 0, mask 0.  v45 tried masking in every stream id: the
	 * write was dropped (SMR0 read back 0, not valid), the GPU's first
	 * accesses then matched nothing and the phone reset.
	 */
	smmu_write(0x800 + 4 * use, (1U << 31) | 0);
	__asm __volatile("dsb sy" : : : "memory");
	s2cr = smmu_read(0xc00 + 4 * use);
	if (((s2cr >> 16) & 3) != 1) {
		vm_paddr_t gr1 = SMMU_BASE + smmu_pgsize;

		smmu_cb = numcb - 1;
		*(volatile uint32_t *)PHYS_TO_DMAP(smmu_cb_base(smmu_cb)) = 0;
		*(volatile uint32_t *)PHYS_TO_DMAP(gr1 + 4 * smmu_cb) =
		    1U << 16;		/* CBAR: S1 translation, S2 bypass */
		smmu_write(0xc00 + 4 * use, (0U << 16) | smmu_cb);
		__asm __volatile("dsb sy" : : : "memory");
		a5.status("a5xx: smmu refuses bypass (S2CR 0x%08x); context "
		    "bank %d of %d with translation off, as Linux's qcom quirk: "
		    "CBAR 0x%08x SCTLR 0x%08x S2CR 0x%08x", s2cr, smmu_cb, numcb,
		    *(volatile uint32_t *)PHYS_TO_DMAP(gr1 + 4 * smmu_cb),
		    *(volatile uint32_t *)PHYS_TO_DMAP(smmu_cb_base(smmu_cb)),
		    smmu_read(0xc00 + 4 * use));
	}
	a5.status("a5xx: smmu sCR0 0x%08x idr1 0x%08x (page %u, %u pages); "
	    "stream 0 in SMR%d (0x%08x / 0x%08x)", scr0, idr1, smmu_pgsize,
	    smmu_numpage, use, smmu_read(0x800 + 4 * use),
	    smmu_read(0xc00 + 4 * use));
	return (0);
}

/* Faults the SMMU recorded, if any. */
static void
smmu_faults(void)
{
	a5.status("a5xx: smmu sGFSR 0x%08x sGFSYNR0 0x%08x sGFSYNR1 0x%08x",
	    smmu_read(0x48), smmu_read(0x50), smmu_read(0x54));
	if (smmu_cb >= 0) {
		volatile uint32_t *cb =
		    (volatile uint32_t *)PHYS_TO_DMAP(smmu_cb_base(smmu_cb));

		a5.status("a5xx: smmu cb%d FSR 0x%08x FAR 0x%08x%08x FSYNR0 "
		    "0x%08x", smmu_cb, cb[0x58 / 4], cb[0x64 / 4], cb[0x60 / 4],
		    cb[0x68 / 4]);
	}
}

static void *
fw_buffer(const uint8_t *data, size_t len, vm_paddr_t *pa)
{
	void *p;

	/* adreno_fw_create_bo: the first dword is a header, not loaded */
	p = contigmalloc(roundup2(len, PAGE_SIZE), M_A5XX, M_WAITOK | M_ZERO,
	    0, 0xffffffffUL, PAGE_SIZE, 0);
	memcpy(p, data + 4, len - 4);
	dcache_wbinv(p, len);
	*pa = vtophys(p);
	return (p);
}

/* a5xx_hw_init's register set-up, the Adreno 530's branches. */
static void
a5xx_regs(void)
{
	uint32_t hbb = A530_HIGHEST_BANK_BIT - 13;
	int i;

	gpu_write(REG_A5XX_VBIF_ROUND_ROBIN_QOS_ARB, 0x00000003);
	gpu_write(REG_A5XX_RBBM_PERFCTR_GPU_BUSY_MASKED, 0xFFFFFFFF);
	gpu_write(REG_A5XX_RBBM_AHB_CNTL0, 0x00000001);
	/* ADRENO_QUIRK_FAULT_DETECT_MASK (the a530's) */
	gpu_write(REG_A5XX_RBBM_INTERFACE_HANG_MASK_CNTL11, 0xF0000000);
	for (i = 1; i <= 7; ++i)
		gpu_write(REG_A5XX_RBBM_INTERFACE_HANG_MASK_CNTL11 + i,
		    0xFFFFFFFF);
	gpu_write(REG_A5XX_RBBM_INTERFACE_HANG_INT_CNTL, (1 << 30) | 0xFFFF);
	gpu_write(REG_A5XX_RBBM_PERFCTR_CNTL, 0x01);
	gpu_write(REG_A5XX_CP_PERFCTR_CP_SEL_0, PERF_CP_ALWAYS_COUNT);
	gpu_write(REG_A5XX_RBBM_PERFCTR_RBBM_SEL_0, 6);
	gpu_write(REG_A5XX_UCHE_CACHE_WAYS, 0x02);
	gpu_write(REG_A5XX_UCHE_TRAP_BASE_LO, 0xFFFF0000);
	gpu_write(REG_A5XX_UCHE_TRAP_BASE_HI, 0x0001FFFF);
	gpu_write(REG_A5XX_UCHE_WRITE_THRU_BASE_LO, 0xFFFF0000);
	gpu_write(REG_A5XX_UCHE_WRITE_THRU_BASE_HI, 0x0001FFFF);
	gpu_write(REG_A5XX_UCHE_GMEM_RANGE_MIN_LO, 0x00100000);
	gpu_write(REG_A5XX_UCHE_GMEM_RANGE_MIN_HI, 0x00000000);
	gpu_write(REG_A5XX_UCHE_GMEM_RANGE_MAX_LO, 0x00100000 + A530_GMEM - 1);
	gpu_write(REG_A5XX_UCHE_GMEM_RANGE_MAX_HI, 0x00000000);
	gpu_write(REG_A5XX_CP_MEQ_THRESHOLDS, 0x40);
	gpu_write(REG_A5XX_CP_MERCIU_SIZE, 0x40);		/* a530 */
	gpu_write(REG_A5XX_CP_ROQ_THRESHOLDS_2, 0x80000060);
	gpu_write(REG_A5XX_CP_ROQ_THRESHOLDS_1, 0x40201B16);
	gpu_write(REG_A5XX_PC_DBG_ECO_CNTL, (0x400 << 11 | 0x300 << 22));
	/* ADRENO_QUIRK_TWO_PASS_USE_WFI (the a530's) */
	gpu_rmw(REG_A5XX_PC_DBG_ECO_CNTL, 0, (1 << 8));
	gpu_write(REG_A5XX_UCHE_MODE_CNTL, 1U << 29);
	gpu_write(REG_A5XX_CP_CHICKEN_DBG, 0x02000000);
	gpu_write(REG_A5XX_RBBM_AHB_CNTL1, 0xA6FFFFFF);
	/* (a5xx_set_hwcg: clock gating, left out for now) */
	gpu_write(REG_A5XX_RBBM_AHB_CNTL2, 0x0000003F);
	gpu_write(REG_A5XX_TPL1_MODE_CNTL, hbb << 7);
	gpu_write(REG_A5XX_RB_MODE_CNTL, hbb << 1);
	gpu_rmw(REG_A5XX_VPC_DBG_ECO_CNTL, 0, (1 << 10));

	gpu_write(REG_A5XX_CP_PROTECT_CNTL, 0x00000007);
	gpu_write(REG_A5XX_CP_PROTECT(0), ADRENO_PROTECT_RW(0x04, 4));
	gpu_write(REG_A5XX_CP_PROTECT(1), ADRENO_PROTECT_RW(0x08, 8));
	gpu_write(REG_A5XX_CP_PROTECT(2), ADRENO_PROTECT_RW(0x10, 16));
	gpu_write(REG_A5XX_CP_PROTECT(3), ADRENO_PROTECT_RW(0x20, 32));
	gpu_write(REG_A5XX_CP_PROTECT(4), ADRENO_PROTECT_RW(0x40, 64));
	gpu_write(REG_A5XX_CP_PROTECT(5), ADRENO_PROTECT_RW(0x80, 64));
	gpu_write(REG_A5XX_CP_PROTECT(6),
	    ADRENO_PROTECT_RW(REG_A5XX_RBBM_SECVID_TSB_TRUSTED_BASE_LO, 16));
	gpu_write(REG_A5XX_CP_PROTECT(7),
	    ADRENO_PROTECT_RW(REG_A5XX_RBBM_SECVID_TRUST_CNTL, 2));
	gpu_write(REG_A5XX_CP_PROTECT(8), ADRENO_PROTECT_RW(0x800, 64));
	gpu_write(REG_A5XX_CP_PROTECT(9), ADRENO_PROTECT_RW(0x840, 8));
	gpu_write(REG_A5XX_CP_PROTECT(10), ADRENO_PROTECT_RW(0x880, 32));
	gpu_write(REG_A5XX_CP_PROTECT(11), ADRENO_PROTECT_RW(0xAA0, 1));
	gpu_write(REG_A5XX_CP_PROTECT(12), ADRENO_PROTECT_RW(0xCC0, 1));
	gpu_write(REG_A5XX_CP_PROTECT(13), ADRENO_PROTECT_RW(0xCF0, 2));
	gpu_write(REG_A5XX_CP_PROTECT(14), ADRENO_PROTECT_RW(0xE68, 8));
	gpu_write(REG_A5XX_CP_PROTECT(15), ADRENO_PROTECT_RW(0xE70, 16));
	gpu_write(REG_A5XX_CP_PROTECT(16), ADRENO_PROTECT_RW(0xE80, 16));
	gpu_write(REG_A5XX_CP_PROTECT(17), ADRENO_PROTECT_RW(0x10000, 0x8000));

	gpu_write(REG_A5XX_RBBM_SECVID_TSB_CNTL, 0);
	gpu_write64(REG_A5XX_RBBM_SECVID_TSB_TRUSTED_BASE_LO, 0x00000000);
	gpu_write(REG_A5XX_RBBM_SECVID_TSB_TRUSTED_SIZE, 0x00000000);

	gpu_write(REG_A5XX_CP_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_VSC_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_GRAS_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_RB_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_PC_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_HLSQ_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_VFD_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_VPC_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_UCHE_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_SP_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_TPL1_ADDR_MODE_CNTL, 0x1);
	gpu_write(REG_A5XX_RBBM_SECVID_TSB_ADDR_MODE_CNTL, 0x1);
}

/*
 * a5xx_pm_resume: power the RB/CCU and SP domains, which the GPMU
 * controls and which are off after the GX GDSC comes up.  Without them
 * the zap shader, a real shader program run by CP_SET_SECURE_MODE,
 * never finishes (v44-v47: the ME stayed busy, nothing else was).
 */
static int
a5xx_domains_on(a5xx_status_fn status)
{
	int i;

	gpu_write(REG_A5XX_GPMU_RBCCU_POWER_CNTL, 0x778000);
	DELAY(3);
	for (i = 0; i < 20 && !(gpu_read(REG_A5XX_GPMU_RBCCU_PWR_CLK_STATUS) &
	    A5XX_GPMU_PWR_ON); ++i)
		DELAY(1);
	gpu_write(REG_A5XX_GPMU_SP_POWER_CNTL, 0x778000);
	for (i = 0; i < 20 && !(gpu_read(REG_A5XX_GPMU_SP_PWR_CLK_STATUS) &
	    A5XX_GPMU_PWR_ON); ++i)
		DELAY(1);
	status("a5xx: RBCCU domain 0x%08x, SP domain 0x%08x (bit 20 = on)",
	    gpu_read(REG_A5XX_GPMU_RBCCU_PWR_CLK_STATUS),
	    gpu_read(REG_A5XX_GPMU_SP_PWR_CLK_STATUS));
	if (!(gpu_read(REG_A5XX_GPMU_RBCCU_PWR_CLK_STATUS) & A5XX_GPMU_PWR_ON) ||
	    !(gpu_read(REG_A5XX_GPMU_SP_PWR_CLK_STATUS) & A5XX_GPMU_PWR_ON))
		return (ETIMEDOUT);
	return (0);
}

/*
 * Registers, microcode, ring and CP_ME_INIT: the part of a5xx_hw_init
 * that is run again after a soft reset.
 */
static int
a5xx_cp_init(a5xx_status_fn status)
{
	int error;

	memset(a5.rb, 0, RB_SIZE);
	dcache_wbinv(a5.rb, RB_SIZE);
	a5.wptr = 0;
	a5xx_regs();
	status("a5xx: registers set, status 0x%08x",
	    gpu_read(REG_A5XX_RBBM_STATUS));

	/* adreno_hw_init / a5xx_hw_init: microcode and ring */
	gpu_write64(REG_A5XX_CP_ME_INSTR_BASE_LO, a5.pm4_pa);
	gpu_write64(REG_A5XX_CP_PFP_INSTR_BASE_LO, a5.pfp_pa);
	gpu_write64(REG_A5XX_CP_RB_BASE, a5.rb_pa);
	gpu_write(REG_A5XX_CP_RB_CNTL, ilog2_u(RB_SIZE / 8) |
	    (ilog2_u(RB_BLKSIZE / 8) << 8) | RB_CNTL_NO_UPDATE);
	gpu_write(REG_A5XX_RBBM_INT_0_MASK, 0);		/* polled */

	/* start the micro engines, then CP_ME_INIT (a5xx_me_init, a530) */
	status("a5xx: microcode and ring set; starting the CP");
	gpu_write(REG_A5XX_CP_PFP_ME_CNTL, 0);
	OUT_PKT7(CP_ME_INIT, 8);
	OUT_RING(0x0000002F);
	OUT_RING(0x00000003);
	OUT_RING(0x20000000);
	OUT_RING(0x00000000);
	OUT_RING(0x00000000);
	OUT_RING(0x0000000B);
	OUT_RING(0x00000000);
	OUT_RING(0x00000000);
	a5xx_flush();
	status("a5xx: CP_ME_INIT queued (wptr %u)", a5.wptr);
	if ((error = a5xx_idle("CP_ME_INIT")) != 0)
		return (error);
	status("a5xx: CP_ME_INIT done: rptr %u, status 0x%08x",
	    gpu_read(REG_A5XX_CP_RB_RPTR), gpu_read(REG_A5XX_RBBM_STATUS));

	return (0);
}

int
a5xx_dfly_start(a5xx_status_fn status, int (*zap)(void), const uint8_t *pm4,
    size_t pm4_len, const uint8_t *pfp, size_t pfp_len)
{
	int error;

	a5.status = status;
	if (pm4_len < 8 || pfp_len < 8)
		return (EINVAL);
	if ((error = smmu_bypass()) != 0)
		return (error);

	a5.pm4 = fw_buffer(pm4, pm4_len, &a5.pm4_pa);
	a5.pfp = fw_buffer(pfp, pfp_len, &a5.pfp_pa);
	a5.pm4_len = pm4_len;
	a5.pfp_len = pfp_len;
	a5.rb = contigmalloc(RB_SIZE, M_A5XX, M_WAITOK | M_ZERO, 0,
	    0xffffffffUL, PAGE_SIZE, 0);
	a5.rb_pa = vtophys(a5.rb);
	a5.scratch = contigmalloc(PAGE_SIZE, M_A5XX, M_WAITOK | M_ZERO, 0,
	    0xffffffffUL, PAGE_SIZE, 0);
	a5.scratch_pa = vtophys(a5.scratch);
	dcache_wbinv(a5.rb, RB_SIZE);
	dcache_wbinv(a5.scratch, PAGE_SIZE);
	a5.wptr = 0;
	status("a5xx: pm4 %zu bytes at 0x%jx, pfp %zu at 0x%jx, ring at "
	    "0x%jx", pm4_len, (uintmax_t)a5.pm4_pa, pfp_len,
	    (uintmax_t)a5.pfp_pa, (uintmax_t)a5.rb_pa);

	if ((error = a5xx_domains_on(status)) != 0)
		return (error);
	if ((error = a5xx_cp_init(status)) != 0)
		return (error);

	/* (a5xx_power_init: the GPMU, left out for now) */

	/*
	 * Out of secure mode, a5xx_hw_init's way: the zap shader is loaded
	 * only now, with the CP running (v44-v46 loaded it before the GPU
	 * was set up, and CP_SET_SECURE_MODE then hung with the ME busy),
	 * and CP_SET_SECURE_MODE has it switch the CP over.  With no zap
	 * shader, RBBM_SECVID_TRUST_CNTL is written directly.
	 */
	if ((error = zap()) == 0) {
		OUT_PKT7(CP_SET_SECURE_MODE, 1);
		OUT_RING(0x00000000);
		a5xx_flush();
		if ((error = a5xx_idle_n("CP_SET_SECURE_MODE", 500000)) != 0)
			return (error);
		status("a5xx: out of secure mode");
	} else {
		status("a5xx: no zap shader (error %d); writing "
		    "RBBM_SECVID_TRUST_CNTL directly", error);
		gpu_write(REG_A5XX_RBBM_SECVID_TRUST_CNTL, 0);
		status("a5xx: RBBM_SECVID_TRUST_CNTL reads 0x%08x",
		    gpu_read(REG_A5XX_RBBM_SECVID_TRUST_CNTL));
	}

	/* the GPU writes a word to memory: commands run */
	a5.scratch[0] = 0;
	dcache_wbinv(a5.scratch, 64);
	OUT_PKT7(CP_MEM_WRITE, 3);
	OUT_RING((uint32_t)a5.scratch_pa);
	OUT_RING((uint32_t)(a5.scratch_pa >> 32));
	OUT_RING(0xC0FFEE53);
	OUT_PKT7(CP_WAIT_FOR_IDLE, 0);
	a5xx_flush();
	if ((error = a5xx_idle("CP_MEM_WRITE")) != 0)
		return (error);
	dcache_wbinv(a5.scratch, 64);
	status("a5xx: CP_MEM_WRITE to 0x%jx: read back 0x%08x (%s)",
	    (uintmax_t)a5.scratch_pa, a5.scratch[0],
	    a5.scratch[0] == 0xC0FFEE53 ? "the GPU runs commands" :
	    "no write seen");
	if (a5.scratch[0] != 0xC0FFEE53)
		return (EIO);
	a5_running = 1;
	return (0);
}

/* ---- after start: submissions, as Linux's a5xx_submit without preemption ---- */

/* Free dwords in the ring (one slot kept empty). */
static uint32_t
ring_space(void)
{
	uint32_t rptr = gpu_read(REG_A5XX_CP_RB_RPTR);

	return ((rptr + RB_SIZE / 4 - a5.wptr - 1) % (RB_SIZE / 4));
}

/*
 * Queue indirect buffers (iova, size in dwords), then write (seqno)
 * to the fence word when the GPU is through them.  The caller has made
 * the buffers visible to the GPU.
 */
int
a5xx_dfly_submit(const uint64_t *iova, const uint32_t *ndw, int n,
    uint32_t seqno)
{
	uint32_t need = 4 * n + 6 + 2 + 5 + 4;
	int i, tries;

	if (!a5_running)
		return (ENXIO);
	for (tries = 0; ring_space() < need; ++tries) {
		if (tries > 100000)
			return (EBUSY);
		DELAY(10);
	}
	for (i = 0; i < n; ++i) {
		OUT_PKT7(CP_INDIRECT_BUFFER_PFE, 3);
		OUT_RING((uint32_t)iova[i]);
		OUT_RING((uint32_t)(iova[i] >> 32));
		OUT_RING(ndw[i]);
	}
	OUT_PKT7(CP_SET_RENDER_MODE, 5);
	OUT_RING(0);
	OUT_RING(0);
	OUT_RING(0);
	OUT_RING(0);
	OUT_RING(0);
	OUT_PKT4(REG_A5XX_CP_SCRATCH_REG(2), 1);
	OUT_RING(seqno);
	OUT_PKT7(CP_EVENT_WRITE, 4);
	OUT_RING(CACHE_FLUSH_TS);
	OUT_RING((uint32_t)(a5.scratch_pa + FENCE_OFFSET));
	OUT_RING((uint32_t)((a5.scratch_pa + FENCE_OFFSET) >> 32));
	OUT_RING(seqno);
	a5xx_flush();
	return (0);
}

/* The last completed sequence number. */
uint32_t
a5xx_dfly_fence(void)
{
	volatile uint32_t *f = (volatile uint32_t *)((char *)a5.scratch +
	    FENCE_OFFSET);

	__asm __volatile("dc ivac, %0; dsb sy" : : "r" (f) : "memory");
	return (*f);
}

uint64_t
a5xx_dfly_timestamp(void)
{
	uint32_t hi, lo, hi2;

	do {
		hi = gpu_read(REG_A5XX_RBBM_ALWAYSON_COUNTER_HI);
		lo = gpu_read(REG_A5XX_RBBM_ALWAYSON_COUNTER_LO);
		hi2 = gpu_read(REG_A5XX_RBBM_ALWAYSON_COUNTER_HI);
	} while (hi != hi2);
	return (((uint64_t)hi << 32) | lo);
}

int
a5xx_dfly_running(void)
{
	return (a5_running);
}
