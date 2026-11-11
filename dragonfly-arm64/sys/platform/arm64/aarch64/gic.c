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
 * ARM Generic Interrupt Controller, versions 2 (GIC-400 and friends)
 * and 3.
 *
 * The two share the distributor's basic register layout.  They differ
 * in the per-cpu half: v2 has a memory mapped cpu interface, v3 has
 * system registers (ICC_*) plus a memory mapped "redistributor" per
 * cpu that holds the state of that cpu's private interrupts.
 *
 * Only the boot cpu is initialized and every shared interrupt is
 * routed to it.  GICv3 LPIs and the ITS (needed for MSI) are not
 * supported.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/globaldata.h>
#include <sys/thread.h>

#include <cpu/armreg.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	GICD_CTLR		0x000
#define	GICD_TYPER		0x004
#define	GICD_ISENABLER(n)	(0x100 + 4 * ((n) / 32))
#define	GICD_ICENABLER(n)	(0x180 + 4 * ((n) / 32))
#define	GICD_IPRIORITYR(n)	(0x400 + (n))
#define	GICD_ITARGETSR(n)	(0x800 + (n))

#define	GICC_CTLR		0x000
#define	GICC_PMR		0x004
#define	GICC_IAR		0x00c
#define	GICC_EOIR		0x010

/* GICv3 distributor additions */
/*
 * Enable bits as Linux sets them: which of bits 0 and 1 is "group 1,
 * non-secure" depends on whether the GIC has two security states (real
 * hardware, seen from the non-secure side) or one (QEMU by default).
 */
#define	GICD_CTLR_ENABLE	0x03
#define	GICD_CTLR_RWP		(1U << 31)	/* register write pending */
#define	GICD_CTLR_ARE_NS	0x10
#define	GICD_IGROUPR(n)		(0x080 + 4 * ((n) / 32))
#define	GICD_IROUTER(n)		(0x6000 + 8 * (n))

/* GICv3 redistributor: two 64KB frames per cpu, RD_base then SGI_base */
#define	GICR_FRAME_SIZE		0x20000
#define	GICR_TYPER		0x008
#define	GICR_TYPER_LAST		0x10
#define	GICR_WAKER		0x014
#define	GICR_WAKER_PSLEEP	0x02	/* ProcessorSleep */
#define	GICR_WAKER_CASLEEP	0x04	/* ChildrenAsleep */
#define	GICR_SGI_BASE		0x10000
#define	GICR_IGROUPR0		(GICR_SGI_BASE + 0x080)
#define	GICR_ISENABLER0		(GICR_SGI_BASE + 0x100)
#define	GICR_ICENABLER0		(GICR_SGI_BASE + 0x180)
#define	GICR_IPRIORITYR(n)	(GICR_SGI_BASE + 0x400 + (n))

/* GICv3 cpu interface system registers, by encoding */
#define	ICC_PMR_EL1		S3_0_C4_C6_0
#define	ICC_IAR1_EL1		S3_0_C12_C12_0
#define	ICC_EOIR1_EL1		S3_0_C12_C12_1
#define	ICC_SRE_EL1		S3_0_C12_C12_5
#define	ICC_IGRPEN1_EL1		S3_0_C12_C12_7
#define	ICC_SGI1R_EL1		S3_0_C12_C11_5

#define	GICD_SGIR		0xf00	/* v2: software generated interrupt */

#define	GIC_SPURIOUS		1023
#define	GIC_IPI_SGI		0	/* the SGI used for lwkt IPIs */
#define	GIC_FIRST_SPI		32

static volatile uint8_t *gicd;
static volatile uint8_t *gicc;		/* v2: cpu interface */
static volatile uint8_t *gicr_base;	/* v3: first redistributor */
static volatile uint8_t *gicr_pcpu[MAXCPU]; /* v3: each cpu's own */
#define	gicr	(gicr_pcpu[mycpuid])
static u_int gicr_stride = GICR_FRAME_SIZE;	/* some SoCs pad the frames */
static int gic_version;
static int gic_nirqs;

static __inline uint32_t
gic_read(volatile uint8_t *base, u_int off)
{
	return (*(volatile uint32_t *)(base + off));
}

static __inline void
gic_write(volatile uint8_t *base, u_int off, uint32_t val)
{
	*(volatile uint32_t *)(base + off) = val;
}

/*
 * GICv3: find the boot cpu's redistributor, wake it, and bring up the
 * system register cpu interface.
 */
/* Wait until a distributor control write has taken effect (v3). */
static void
gic_v3_wait_rwp(void)
{
	int n;

	for (n = 0; n < 1000000; ++n) {
		if ((gic_read(gicd, GICD_CTLR) & GICD_CTLR_RWP) == 0)
			return;
		cpu_pause();
	}
}

/*
 * GICv3, per cpu: find this cpu's redistributor, wake it, set up its
 * private interrupts and the system register cpu interface.
 */
static void
gic_v3_init_cpu(void)
{
	uint64_t mpidr = READ_SPECIALREG(mpidr_el1);
	uint32_t aff = ((mpidr >> 8) & 0xff000000) | (mpidr & 0x00ffffff);
	volatile uint8_t *rd;
	uint64_t typer;
	int irq, n;

	/* Each cpu's frames carry its affinity in GICR_TYPER[63:32]. */
	boot_mark("boot: gic: finding redistributor\n");
	rd = gicr_base;
	gicr = rd;
	for (n = 0; n < 256; ++n) {
		typer = *(volatile uint64_t *)(rd + GICR_TYPER);
		if ((uint32_t)(typer >> 32) == aff) {
			gicr = rd;
			break;
		}
		if (typer & GICR_TYPER_LAST)
			break;
		rd += gicr_stride;
	}

	/*
	 * Mark the cpu awake and wait for the redistributor to agree.
	 * Some firmware owns this register, and the bootloader (for the
	 * boot cpu) or PSCI CPU_ON (for the others) has done it already
	 * (see qcom_msm8996.c).
	 */
	if (!arm64_bootinfo.bi_gic_no_waker) {
		boot_mark("boot: gic: waking redistributor\n");
		gic_write(gicr, GICR_WAKER,
		    gic_read(gicr, GICR_WAKER) & ~GICR_WAKER_PSLEEP);
		for (n = 0; n < 1000000; ++n) {
			if ((gic_read(gicr, GICR_WAKER) &
			    GICR_WAKER_CASLEEP) == 0)
				break;
			cpu_pause();
		}
	}
	boot_mark("boot: gic: private interrupts\n");

	/* Private interrupts (SGIs and PPIs): group 1, off, default prio. */
	gic_write(gicr, GICR_IGROUPR0, 0xffffffff);
	gic_write(gicr, GICR_ICENABLER0, 0xffffffff);
	for (irq = 0; irq < GIC_FIRST_SPI; ++irq)
		gicr[GICR_IPRIORITYR(irq)] = 0x80;

	/*
	 * The cpu interface.  As Linux does, only write ICC_SRE_EL1 if
	 * the system register interface is not on already: a hypervisor
	 * may trap the write.
	 */
	boot_mark("boot: gic: cpu interface\n");
	if ((READ_SPECIALREG(ICC_SRE_EL1) & 1) == 0) {
		WRITE_SPECIALREG(ICC_SRE_EL1, READ_SPECIALREG(ICC_SRE_EL1) | 1);
		__asm __volatile("isb");
	}
	WRITE_SPECIALREG(ICC_PMR_EL1, 0xff);
	WRITE_SPECIALREG(ICC_IGRPEN1_EL1, 1);
	__asm __volatile("isb");
}

/*
 * GICv3, once: the distributor.  Shared interrupts are group 1 and
 * routed to the boot cpu.
 */
static void
gic_v3_init(vm_paddr_t gicr_pa)
{
	uint64_t mpidr = READ_SPECIALREG(mpidr_el1);
	int irq;

	gicr_base = (volatile uint8_t *)PHYS_TO_DMAP(gicr_pa);

	boot_mark("boot: gic: distributor\n");
	gic_write(gicd, GICD_CTLR, 0);
	gic_v3_wait_rwp();
	for (irq = GIC_FIRST_SPI; irq < gic_nirqs; irq += 32) {
		gic_write(gicd, GICD_IGROUPR(irq), 0xffffffff);
		gic_write(gicd, GICD_ICENABLER(irq), 0xffffffff);
	}
	for (irq = GIC_FIRST_SPI; irq < gic_nirqs; ++irq) {
		gicd[GICD_IPRIORITYR(irq)] = 0x80;
		*(volatile uint64_t *)(gicd + GICD_IROUTER(irq)) =
		    mpidr & MPIDR_AFF_MASK;
	}
	gic_write(gicd, GICD_CTLR, GICD_CTLR_ARE_NS | GICD_CTLR_ENABLE);
	gic_v3_wait_rwp();

	gic_v3_init_cpu();
}

/*
 * Interrupt controller setup on a secondary cpu: its private
 * interrupts, its cpu interface and the IPI line.
 */
void
gic_init_secondary(void)
{
	int irq;

	if (gicd == NULL)
		return;
	if (gic_version == 3) {
		gic_v3_init_cpu();
	} else {
		/* Private interrupt state is banked per cpu. */
		gic_write(gicd, GICD_ICENABLER(0), 0xffffffff);
		for (irq = 0; irq < GIC_FIRST_SPI; ++irq)
			gicd[GICD_IPRIORITYR(irq)] = 0x80;
		gic_write(gicc, GICC_PMR, 0xff);
		gic_write(gicc, GICC_CTLR, 1);
	}
	gic_enable_irq(GIC_IPI_SGI);
}

/*
 * Raise the IPI on another cpu.  (mpidr) is its hardware affinity.
 */
void
gic_send_ipi(int cpu, u_long mpidr)
{
	if (gicd == NULL)
		return;
	cpu_mfence();		/* the message must be visible first */
	if (gic_version == 3) {
		uint64_t val;

		/* Aff3.Aff2.Aff1 pick the cluster, a bit the cpu in it. */
		val = ((mpidr >> 32) & 0xff) << 48 |
		    ((mpidr >> 16) & 0xff) << 32 |
		    ((mpidr >> 8) & 0xff) << 16 |
		    (uint64_t)GIC_IPI_SGI << 24 |
		    1UL << (mpidr & 0xf);
		WRITE_SPECIALREG(ICC_SGI1R_EL1, val);
		__asm __volatile("isb");
	} else {
		/* v2 addresses cpus by interface number: assume == cpu id */
		gic_write(gicd, GICD_SGIR, (1U << (16 + cpu)) | GIC_IPI_SGI);
	}
}

/*
 * Returns the number of interrupt lines the distributor implements.
 * (percpu_pa) is the cpu interface for version 2 and the first
 * redistributor for version 3.
 */
int
gic_init(int version, vm_paddr_t gicd_pa, vm_paddr_t percpu_pa)
{
	int irq;

	gic_version = version;
	gicd = (volatile uint8_t *)PHYS_TO_DMAP(gicd_pa);
	gic_nirqs = 32 * ((gic_read(gicd, GICD_TYPER) & 0x1f) + 1);
	if (gic_nirqs > 1020)
		gic_nirqs = 1020;

	if (version == 3) {
		if (arm64_bootinfo.bi_gicr_stride != 0)
			gicr_stride = arm64_bootinfo.bi_gicr_stride;
		gic_v3_init(percpu_pa);
		gic_enable_irq(GIC_IPI_SGI);
		return (gic_nirqs);
	}

	gicc = (volatile uint8_t *)PHYS_TO_DMAP(percpu_pa);

	/* Everything off, default priority, shared interrupts to cpu 0. */
	for (irq = 0; irq < gic_nirqs; irq += 32)
		gic_write(gicd, GICD_ICENABLER(irq), 0xffffffff);
	for (irq = 0; irq < gic_nirqs; ++irq) {
		gicd[GICD_IPRIORITYR(irq)] = 0x80;
		if (irq >= GIC_FIRST_SPI)
			gicd[GICD_ITARGETSR(irq)] = 0x01;
	}

	gic_write(gicd, GICD_CTLR, 1);		/* enable distributor */
	gic_write(gicc, GICC_PMR, 0xff);	/* unmask all priorities */
	gic_write(gicc, GICC_CTLR, 1);		/* enable cpu interface */
	gic_enable_irq(GIC_IPI_SGI);
	return (gic_nirqs);
}

void
gic_enable_irq(int irq)
{
	if (gicd == NULL || irq >= gic_nirqs)
		return;
	if (gic_version == 3 && irq < GIC_FIRST_SPI)
		gic_write(gicr, GICR_ISENABLER0, 1U << irq);
	else
		gic_write(gicd, GICD_ISENABLER(irq), 1U << (irq % 32));
}

void
gic_disable_irq(int irq)
{
	if (gicd == NULL || irq >= gic_nirqs)
		return;
	if (gic_version == 3 && irq < GIC_FIRST_SPI)
		gic_write(gicr, GICR_ICENABLER0, 1U << irq);
	else
		gic_write(gicd, GICD_ICENABLER(irq), 1U << (irq % 32));
}

/*
 * Acknowledge the highest priority pending interrupt.  Returns the raw
 * IAR value (to be handed back to gic_eoi()) or -1 if there is none.
 */
int
gic_ack(void)
{
	uint32_t iar;

	if (gicd == NULL)
		return (-1);
	if (gic_version == 3)
		iar = (uint32_t)READ_SPECIALREG(ICC_IAR1_EL1);
	else
		iar = gic_read(gicc, GICC_IAR);
	if ((iar & 0x3ff) >= 1020)
		return (-1);
	return ((int)iar);
}

void
gic_eoi(int iar)
{
	if (gic_version == 3)
		WRITE_SPECIALREG(ICC_EOIR1_EL1, (uint32_t)iar);
	else
		gic_write(gicc, GICC_EOIR, (uint32_t)iar);
}
