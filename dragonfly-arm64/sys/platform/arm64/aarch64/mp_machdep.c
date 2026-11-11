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
 * Multiprocessor support: starting the secondary cpus and talking to
 * them.
 *
 * Secondary cpus are started with PSCI CPU_ON, a firmware call that
 * powers a cpu up and sends it to a physical address with the MMU off
 * (mpentry, in locore.S).  From there it switches the MMU on with the
 * boot page tables, calls init_secondary() for its per-cpu hardware,
 * and becomes its own idle thread, whose first act is ap_init().
 *
 * The handshake with the boot cpu follows pc64's mp_machdep.c:
 *
 *   boot cpu (arm64_mp_enumerate, SI_BOOT2_LAPIC)
 *	count the cpus in the device tree and set naps, which the cpu
 *	topology (SI_BOOT2_CPU_TOPOLOGY) is built from
 *   boot cpu (arm64_mp_start, SI_BOOT2_START_APS)
 *	set ncpus, allocate each cpu's private data and idle thread,
 *	CPU_ON it, wait for its startup bit
 *   each secondary (ap_init)
 *	set its bit in smp_startup_mask, wait for the boot cpu to let
 *	go of the MP lock, wait for mp_finish, start taking interrupts,
 *	start its clock, set its bit in smp_finalize_mask
 *   boot cpu (ap_finish, SI_BOOT2_FINISH_SMP)
 *	release the MP lock, raise mp_finish, wait for everyone
 *
 * Inter-processor interrupts are GIC software generated interrupts
 * (gic.c).  TLB invalidations need no IPIs: the pmap uses the
 * broadcast forms of TLBI.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/malloc.h>
#include <sys/libkern.h>
#include <sys/globaldata.h>
#include <sys/thread.h>
#include <sys/mplock2.h>

#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/vm_kern.h>
#include <vm/pmap.h>
#include <vm/vm_extern.h>

#include <sys/thread2.h>

#include <machine/cpufunc.h>
#include <machine/globaldata.h>
#include <machine/md_var.h>
#include <machine/smp.h>

#define	PSCI_CPU_ON_64		0xc4000003UL
#define	PSCI_E_ALREADY_ON	(-4)

volatile cpumask_t stopped_cpus;
cpumask_t	smp_active_mask = CPUMASK_INITIALIZER_ONLYONE;
int		naps;			/* application processors started */

static cpumask_t smp_startup_mask = CPUMASK_INITIALIZER_ONLYONE;
static cpumask_t smp_finalize_mask = CPUMASK_INITIALIZER_ONLYONE;
static volatile int mp_finish;
static u_long	cpu_mpidr[MAXCPU];	/* hardware id of each logical cpu */
static int	mp_want;		/* cpus to run, boot cpu included */

/*
 * Order the cpus: the boot cpu is cpu 0, the others follow in device
 * tree order.  Returns how many there are (at least 1).
 */
static int
mp_order_cpus(u_long *mpidr)
{
	struct arm64_bootinfo *bi = &arm64_bootinfo;
	u_long self = READ_SPECIALREG(mpidr_el1) & MPIDR_AFF_MASK;
	int n = 1, i, max;

	mpidr[0] = self;
	if (bi->bi_psci == 0)
		return (1);		/* no way to start anyone */
	max = bi->bi_ncpus;
	if (max > ARM64_MAXFDTCPUS)
		max = ARM64_MAXFDTCPUS;
	for (i = 0; i < max && n < MAXCPU; ++i) {
		if ((bi->bi_cpu_mpidr[i] & MPIDR_AFF_MASK) == self)
			continue;
		mpidr[n++] = bi->bi_cpu_mpidr[i] & MPIDR_AFF_MASK;
	}
	return (n);
}

/*
 * Decide how many cpus to run.  Boot arguments can lower it:
 * cpus=N (total) or hw.ap_max=N (secondaries).
 */
static void
arm64_mp_enumerate(void *dummy __unused)
{
	int avail, n;

	avail = mp_order_cpus(cpu_mpidr);
	mp_want = avail;
	n = -1;
	TUNABLE_INT_FETCH("hw.ap_max", &n);
	if (n >= 0)
		mp_want = n + 1;
	n = 0;
	if (kgetenv_int("cpus", &n) && n >= 1)
		mp_want = n;
	if (mp_want > avail)
		mp_want = avail;
	if (mp_want < 1)
		mp_want = 1;
	naps = mp_want - 1;
}
SYSINIT(mpenum, SI_BOOT2_LAPIC, SI_ORDER_FIRST, arm64_mp_enumerate, NULL);

/*
 * Ask the firmware to start the cpu with hardware id (mpidr) at
 * mpentry, which receives (cpu) in x0.
 */
static int
psci_cpu_on(u_long mpidr, int cpu)
{
	vm_paddr_t entry = pmap_kextract((vm_offset_t)mpentry);

	return ((int)psci_call4(PSCI_CPU_ON_64, mpidr, entry, cpu,
	    arm64_bootinfo.bi_psci == 2));
}

/*
 * Set up and start the secondary cpus.  Runs on the boot cpu at the
 * point in boot where pc64 starts its application processors; also
 * where every cpu, the boot cpu included, gets its IPI queues.
 */
static void
arm64_mp_start(void *dummy __unused)
{
	u_long *mpidr = cpu_mpidr;
	struct mdglobaldata *gd;
	struct privatespace *ps;
	size_t ipiq_size;
	int shift, x, err, waited;

	ncpus = mp_want;
	for (shift = 0; (1 << shift) < ncpus; ++shift)
		;
	ncpus_fit = 1 << shift;
	ncpus_fit_mask = ncpus_fit - 1;
	malloc_reinit_ncpus();

	ipiq_size = sizeof(struct lwkt_ipiq) * ncpus;
	mycpu->gd_ipiq = (void *)kmem_alloc(kernel_map, ipiq_size,
					    VM_SUBSYS_IPIQ);
	bzero(mycpu->gd_ipiq, ipiq_size);
	arc4_init_pcpu(0);
	mdcpu->gd_mpidr = mpidr[0];

	for (x = 1; x < ncpus; ++x) {
		ps = (struct privatespace *)kmem_alloc(kernel_map,
		    sizeof(struct privatespace), VM_SUBSYS_GD);
		bzero(ps, sizeof(*ps));
		CPU_prvspace[x] = ps;
		gd = (struct mdglobaldata *)ps;	/* first member */
		gd->mi.gd_prvspace = ps;
		gd->gd_mpidr = mpidr[x];
		mi_gdinit(&gd->mi, x);
		cpu_gdinit(gd, x);
		gd->mi.gd_ipiq = (void *)kmem_alloc(kernel_map, ipiq_size,
						    VM_SUBSYS_IPIQ);
		bzero(gd->mi.gd_ipiq, ipiq_size);
		arc4_init_pcpu(x);
		cpu_mfence();

		err = psci_cpu_on(mpidr[x], x);
		if (err != 0 && err != PSCI_E_ALREADY_ON) {
			kprintf("cpu%d (mpidr %lx): PSCI CPU_ON failed (%d)\n",
			    x, mpidr[x], err);
			break;
		}
		for (waited = 0; waited < 1000; ++waited) {
			if (CPUMASK_TESTBIT(smp_startup_mask, x))
				break;
			DELAY(1000);
		}
		if (!CPUMASK_TESTBIT(smp_startup_mask, x)) {
			kprintf("cpu%d (mpidr %lx): did not start\n",
			    x, mpidr[x]);
			break;
		}
	}
	if (x < ncpus) {
		/*
		 * Run with the cpus that did start.  Their structures were
		 * sized for more, which wastes a little memory only.
		 */
		ncpus = x;
		for (shift = 0; (1 << shift) < ncpus; ++shift)
			;
		ncpus_fit = 1 << shift;
		ncpus_fit_mask = ncpus_fit - 1;
	}
	naps = ncpus - 1;
	if (ncpus > 1)
		kprintf("SMP: %d cpus\n", ncpus);
}
SYSINIT(startaps, SI_BOOT2_START_APS, SI_ORDER_FIRST, arm64_mp_start, NULL);

/*
 * Per-cpu hardware setup on a secondary cpu, called from mpentry with
 * the MMU on, on the cpu's boot stack.
 */
void
init_secondary(int cpu)
{
	KKASSERT(mycpu->gd_cpuid == cpu);

	WRITE_SPECIALREG(cntv_ctl_el0, 0);	/* no stale timer */
	gic_init_secondary();
	gic_enable_irq(arm64_bootinfo.bi_timer_irq);
	if (bootverbose) {
		kprintf("cpu%d: started, mpidr %lx\n", cpu,
		    READ_SPECIALREG(mpidr_el1) & MPIDR_AFF_MASK);
	}
}

/*
 * First thing a secondary cpu's idle thread does: join the system.
 * Entered with interrupts masked, in the idle thread's critical
 * section.
 */
void
ap_init(void)
{
	/* Tell the boot cpu we are alive. */
	ATOMIC_CPUMASK_ORBIT(smp_startup_mask, mycpu->gd_cpuid);
	cpu_mfence();

	/* The boot cpu holds the MP lock until ap_finish(). */
	while (try_mplock() == 0) {
		cpu_pause();
		cpu_lfence();
	}
	cpu_invltlb();
	mycpu->gd_other_cpus = smp_startup_mask;
	ATOMIC_CPUMASK_NANDBIT(mycpu->gd_other_cpus, mycpu->gd_cpuid);
	cpu_mfence();
	rel_mplock();

	while (mp_finish == 0) {
		cpu_pause();
		cpu_lfence();
	}
	cpu_invltlb();

	/* Start taking interrupts, IPIs included. */
	crit_enter();
	cpu_enable_intr();
	bzero(mdcpu->gd_ipending, sizeof(mdcpu->gd_ipending));
	ATOMIC_CPUMASK_ORBIT(smp_active_mask, mycpu->gd_cpuid);

	while (mp_finish == 1) {
		cpu_pause();
		cpu_lfence();
	}
	cpu_invltlb();

	initclocks_pcpu();	/* this cpu's clock interrupt */
	atomic_swap_int(&mycpu->gd_npoll, 0);
	lwkt_process_ipiq();
	crit_exit();

	ATOMIC_CPUMASK_ORBIT(smp_finalize_mask, mycpu->gd_cpuid);
	KKASSERT((curthread->td_flags & TDF_RUNQ) == 0);
}

/*
 * Let the secondary cpus go, once the kernel is far enough along for
 * them to schedule threads.  Runs on the boot cpu.
 */
static void
ap_finish(void *dummy __unused)
{
	mycpu->gd_other_cpus = smp_startup_mask;
	ATOMIC_CPUMASK_NANDBIT(mycpu->gd_other_cpus, mycpu->gd_cpuid);

	rel_mplock();
	mp_finish = 1;
	while (CPUMASK_CMPMASKNEQ(smp_active_mask, smp_startup_mask)) {
		cpu_pause();
		cpu_lfence();
	}
	cpu_invltlb();
	mp_finish = 2;
	while (CPUMASK_CMPMASKNEQ(smp_finalize_mask, smp_startup_mask)) {
		cpu_pause();
		cpu_lfence();
	}
	while (try_mplock() == 0) {
		cpu_pause();
		cpu_lfence();
	}
	if (bootverbose && ncpus > 1) {
		kprintf("SMP: active cpu mask %016jx\n",
		    (uintmax_t)CPUMASK_LOWMASK(smp_active_mask));
	}
}
SYSINIT(finishsmp, SI_BOOT2_FINISH_SMP, SI_ORDER_FIRST, ap_finish, NULL);

/*
 * Inter-processor interrupts
 */

void
cpu_send_ipiq(int dcpu)
{
	struct mdglobaldata *gd;

	if (CPUMASK_TESTBIT(smp_active_mask, dcpu)) {
		gd = (struct mdglobaldata *)globaldata_find(dcpu);
		gic_send_ipi(dcpu, gd->gd_mpidr);
	}
}

int
cpu_send_ipiq_passive(int dcpu)
{
	cpu_send_ipiq(dcpu);
	return (0);
}

/*
 * TLB invalidations are broadcast in hardware (TLBI ...IS).
 */
void
smp_invltlb(void)
{
	__asm __volatile(
	    "dsb	ishst\n"
	    "tlbi	vmalle1is\n"
	    "dsb	ish\n"
	    "isb" : : : "memory");
}

void
smp_sniff(void)
{
}

void
cpu_sniff(int dcpu __unused)
{
}

/*
 * Stopping the other cpus (for the debugger and for panics) needs a
 * second, non-maskable-like IPI; not done yet.  They keep running.
 */
int
stop_cpus(cpumask_t map __unused)
{
	return (1);
}

int
restart_cpus(cpumask_t map __unused)
{
	return (1);
}

void
cpu_smp_stopped(void)
{
}
