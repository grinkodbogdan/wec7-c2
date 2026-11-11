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
 * Interrupt dispatch, the machine interrupt ABI and soft interrupts.
 *
 * DragonFly runs interrupt handlers in threads.  The job here is to
 * get from "the cpu took an IRQ exception" to sched_ithd_hard(irq),
 * honoring critical sections: while the interrupted thread is in one,
 * the interrupt is only recorded (and its source masked) and splz()
 * delivers it when the critical section ends.  On pc64 this lives in
 * assembly (ipl.s, apic_vector.s); the logic is the same.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/interrupt.h>
#include <sys/machintr.h>
#include <sys/rman.h>
#include <sys/bus.h>
#include <sys/sysctl.h>

#include <sys/thread2.h>

#include <machine/cpu.h>
#include <machine/globaldata.h>
#include <machine/md_var.h>

static void	gic_abi_intr_disable(int);
static void	gic_abi_intr_enable(int);
static void	gic_abi_intr_setup(int, int);
static void	gic_abi_intr_teardown(int);
static void	gic_abi_legacy_intr_config(int, enum intr_trigger,
		    enum intr_polarity);
static int	gic_abi_legacy_intr_cpuid(int);
static int	gic_abi_legacy_intr_find(int, enum intr_trigger,
		    enum intr_polarity);
static int	gic_abi_legacy_intr_find_bygsi(int, enum intr_trigger,
		    enum intr_polarity);
static int	gic_abi_msi_alloc(int [], int, int);
static void	gic_abi_msi_release(const int [], int, int);
static void	gic_abi_msi_map(int, uint64_t *, uint32_t *, int);
static int	gic_abi_msix_alloc(int *, int);
static void	gic_abi_msix_release(int, int);
static void	gic_abi_nop(void);
static void	gic_abi_rman_setup(struct rman *);

struct machintr_abi MachIntrABI = {
	MACHINTR_GENERIC,
	.intr_disable	= gic_abi_intr_disable,
	.intr_enable	= gic_abi_intr_enable,
	.intr_setup	= gic_abi_intr_setup,
	.intr_teardown	= gic_abi_intr_teardown,

	.legacy_intr_config = gic_abi_legacy_intr_config,
	.legacy_intr_cpuid = gic_abi_legacy_intr_cpuid,
	.legacy_intr_find = gic_abi_legacy_intr_find,
	.legacy_intr_find_bygsi = gic_abi_legacy_intr_find_bygsi,

	.msi_alloc	= gic_abi_msi_alloc,
	.msi_release	= gic_abi_msi_release,
	.msi_map	= gic_abi_msi_map,
	.msix_alloc	= gic_abi_msix_alloc,
	.msix_release	= gic_abi_msix_release,

	.finalize	= gic_abi_nop,
	.cleanup	= gic_abi_nop,
	.setdefault	= gic_abi_nop,
	.stabilize	= gic_abi_nop,
	.initmap	= gic_abi_nop,
	.rman_setup	= gic_abi_rman_setup
};

static int	arm64_nirqs;

/* IPIs each cpu has taken, for checking that they arrive at all. */
static u_long	arm64_ipis[MAXCPU];
SYSCTL_OPAQUE(_machdep, OID_AUTO, ipis, CTLFLAG_RD, arm64_ipis,
    sizeof(arm64_ipis), "LU", "IPIs received, per cpu");

static void
gic_abi_intr_disable(int irq)
{
	gic_disable_irq(irq);
}

static void
gic_abi_intr_enable(int irq)
{
	gic_enable_irq(irq);
}

static void
gic_abi_intr_setup(int irq, int flags __unused)
{
	gic_enable_irq(irq);
}

static void
gic_abi_intr_teardown(int irq)
{
	gic_disable_irq(irq);
}

static void
gic_abi_legacy_intr_config(int irq __unused, enum intr_trigger trig __unused,
			   enum intr_polarity pola __unused)
{
}

static int
gic_abi_legacy_intr_cpuid(int irq __unused)
{
	return (0);
}

static int
gic_abi_legacy_intr_find(int irq, enum intr_trigger trig __unused,
			 enum intr_polarity pola __unused)
{
	return (irq >= 0 && irq < arm64_nirqs ? irq : -1);
}

static int
gic_abi_legacy_intr_find_bygsi(int gsi, enum intr_trigger trig,
			       enum intr_polarity pola)
{
	return (gic_abi_legacy_intr_find(gsi, trig, pola));
}

/* Message signalled interrupts need a GICv2m frame or a GICv3 ITS. */
static int
gic_abi_msi_alloc(int intrs[] __unused, int count __unused, int cpuid __unused)
{
	return (ENXIO);
}

static void
gic_abi_msi_release(const int intrs[] __unused, int count __unused,
		    int cpuid __unused)
{
}

static void
gic_abi_msi_map(int intr __unused, uint64_t *addr __unused,
		uint32_t *data __unused, int cpuid __unused)
{
	panic("gic_abi_msi_map: MSI is not supported");
}

static int
gic_abi_msix_alloc(int *intr __unused, int cpuid __unused)
{
	return (ENXIO);
}

static void
gic_abi_msix_release(int intr __unused, int cpuid __unused)
{
}

static void
gic_abi_nop(void)
{
}

static void
gic_abi_rman_setup(struct rman *rm)
{
	if (arm64_nirqs > 0 &&
	    rman_manage_region(rm, 0, arm64_nirqs - 1) != 0)
		panic("gic_abi_rman_setup: rman_manage_region failed");
}

/*
 * Bring up the interrupt controller found in the device tree.
 */
void
arm64_intr_init(void)
{
	struct arm64_bootinfo *bi = &arm64_bootinfo;

	if ((bi->bi_gic_version != 2 && bi->bi_gic_version != 3) ||
	    bi->bi_gicc_base == 0) {
		kprintf("WARNING: no supported interrupt controller "
		    "(GIC version %d), running without interrupts\n",
		    bi->bi_gic_version);
		return;
	}
	arm64_nirqs = gic_init(bi->bi_gic_version, bi->bi_gicd_base,
	    bi->bi_gicc_base);
	if (arm64_nirqs > MAX_HARDINTS)
		arm64_nirqs = MAX_HARDINTS;
	kprintf("gic0: GICv%d, distributor 0x%jx, %s 0x%jx, %d irqs\n",
	    bi->bi_gic_version, (uintmax_t)bi->bi_gicd_base,
	    bi->bi_gic_version == 3 ? "redistributors" : "cpu interface",
	    (uintmax_t)bi->bi_gicc_base, arm64_nirqs);
}

/*
 * IRQ exception entry, called from the vector with interrupts masked.
 */
void
arm64_intr(struct trapframe *frame)
{
	struct mdglobaldata *gd = mdcpu;
	thread_t td = gd->mi.gd_curthread;
	int iar, irq;

	while ((iar = gic_ack()) != -1) {
		irq = iar & 0x3ff;
		if (arm64_nirqs == 0) {
			/* Should not happen now the GIC starts early. */
			gic_disable_irq(irq);
			gic_eoi(iar);
			continue;
		}

		if (irq == arm64_bootinfo.bi_timer_irq) {
			/*
			 * The timer's output is level triggered: silence
			 * it before signalling end-of-interrupt.  The
			 * next reload re-enables it.
			 */
			clock_intr_quiesce();
			gic_eoi(iar);
			if (td->td_critcount || td->td_nest_count) {
				atomic_set_int(&gd->mi.gd_reqflags, RQF_TIMER);
			} else {
				crit_enter_quick(td);
				++gd->mi.gd_intr_nesting_level;
				clock_intr((struct intrframe *)frame);
				--gd->mi.gd_intr_nesting_level;
				crit_exit_noyield(td);
			}
			continue;
		}

		if (irq < 16) {
			/* An IPI from another cpu: messages are waiting. */
			gic_eoi(iar);
			++arm64_ipis[gd->mi.gd_cpuid];
			if (td->td_critcount || td->td_nest_count) {
				atomic_set_int(&gd->mi.gd_reqflags, RQF_IPIQ);
			} else {
				crit_enter_quick(td);
				++gd->mi.gd_intr_nesting_level;
				atomic_swap_int(&gd->mi.gd_npoll, 0);
				lwkt_process_ipiq_frame(
				    (struct intrframe *)frame);
				--gd->mi.gd_intr_nesting_level;
				crit_exit_noyield(td);
			}
			continue;
		}

		/*
		 * A device interrupt.  Mask the source until its handler
		 * thread has run; kern_intr.c unmasks it again through
		 * MachIntrABI.intr_enable().
		 */
		gic_disable_irq(irq);
		gic_eoi(iar);
		if (irq >= MAX_HARDINTS) {
			kprintf("stray interrupt %d\n", irq);
			continue;
		}
		if (td->td_critcount || td->td_nest_count) {
			atomic_set_long(&gd->gd_ipending[irq >> 6],
			    1UL << (irq & 63));
			atomic_set_int(&gd->mi.gd_reqflags, RQF_INTPEND);
		} else {
			++td->td_nest_count;
			++gd->mi.gd_intr_nesting_level;
			sched_ithd_hard(irq);
			--gd->mi.gd_intr_nesting_level;
			--td->td_nest_count;
		}
	}

	/* Deliver whatever became pending while we were in here. */
	if (td->td_critcount == 0 && td->td_nest_count == 0 &&
	    (gd->mi.gd_reqflags & (RQF_IPIQ | RQF_INTPEND | RQF_TIMER)))
		splz();
}

/*
 * Run interrupts that were deferred because the current thread was in
 * a critical section.  Called when the critical section ends.
 */
void
splz(void)
{
	struct mdglobaldata *gd = mdcpu;
	thread_t td = gd->mi.gd_curthread;
	int irq, i;

	while (gd->mi.gd_reqflags & (RQF_IPIQ | RQF_INTPEND | RQF_TIMER)) {
		crit_enter_quick(td);
		if (gd->mi.gd_reqflags & RQF_IPIQ) {
			atomic_clear_int(&gd->mi.gd_reqflags, RQF_IPIQ);
			atomic_swap_int(&gd->mi.gd_npoll, 0);
			lwkt_process_ipiq();
		}
		if (gd->mi.gd_reqflags & RQF_INTPEND) {
			atomic_clear_int(&gd->mi.gd_reqflags, RQF_INTPEND);
			for (i = 0; i < (int)NELEM(gd->gd_ipending); ++i) {
				while (gd->gd_ipending[i] != 0) {
					irq = bsfq(gd->gd_ipending[i]);
					atomic_clear_long(&gd->gd_ipending[i],
					    1UL << irq);
					sched_ithd_hard(irq + i * 64);
				}
			}
			while ((irq = ffs(gd->gd_spending)) != 0) {
				--irq;
				atomic_clear_int(&gd->gd_spending, 1 << irq);
				sched_ithd_soft(FIRST_SOFTINT + irq);
			}
		}
		if (gd->mi.gd_reqflags & RQF_TIMER) {
			atomic_clear_int(&gd->mi.gd_reqflags, RQF_TIMER);
			clock_intr(NULL);
		}
		crit_exit_noyield(td);
	}
}

/*
 * Returns non-zero if the thread is, or is about to be, held up by
 * interrupt processing.
 */
int
cpu_interrupt_running(struct thread *td)
{
	struct mdglobaldata *gd = mdcpu;

	if ((td->td_flags & TDF_INTTHREAD) || gd->gd_spending ||
	    gd->gd_ipending[0] || gd->gd_ipending[1] || gd->gd_ipending[2])
		return (1);
	return (0);
}

/*
 * Soft interrupts: mark one pending for splz() to schedule.
 */
#define DO_SETBITS(name, var, bits) 					\
void									\
name(void)								\
{									\
	struct mdglobaldata *gd = mdcpu;				\
	atomic_set_int_nonlocked(var, bits);				\
	atomic_set_int(&gd->mi.gd_reqflags, RQF_INTPEND);		\
}									\

DO_SETBITS(setdelayed,   &gd->gd_spending, loadandclear(&gd->gd_sdelayed))

DO_SETBITS(setsoftcamnet,&gd->gd_spending, SWI_CAMNET_PENDING)
DO_SETBITS(setsoftcambio,&gd->gd_spending, SWI_CAMBIO_PENDING)
DO_SETBITS(setsofttty,   &gd->gd_spending, SWI_TTY_PENDING)
DO_SETBITS(setsoftvm,	 &gd->gd_spending, SWI_VM_PENDING)
DO_SETBITS(setsofttq,	 &gd->gd_spending, SWI_TQ_PENDING)
DO_SETBITS(setsoftcrypto,&gd->gd_spending, SWI_CRYPTO_PENDING)

DO_SETBITS(schedsofttty, &gd->gd_sdelayed, SWI_TTY_PENDING)
