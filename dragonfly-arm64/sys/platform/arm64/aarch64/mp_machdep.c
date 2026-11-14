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
 *	set its bit in smp_startup_mask, sleep (WFE) until mp_finish,
 *	take and drop the MP lock, start taking interrupts, start its
 *	clock, set its bit in smp_finalize_mask
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
#include <sys/sysctl.h>

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

#define	PSCI_VERSION		0x84000000UL
#define	PSCI_CPU_ON_64		0xc4000003UL
#define	PSCI_AFFINITY_INFO_64	0xc4000004UL
#define	PSCI_E_ALREADY_ON	(-4)

volatile cpumask_t stopped_cpus;
cpumask_t	smp_active_mask = CPUMASK_INITIALIZER_ONLYONE;
int		naps;			/* application processors started */

static cpumask_t smp_startup_mask = CPUMASK_INITIALIZER_ONLYONE;
static cpumask_t smp_finalize_mask = CPUMASK_INITIALIZER_ONLYONE;
static volatile int mp_finish;
static u_long	cpu_mpidr[MAXCPU];	/* hardware id of each logical cpu */

/*
 * What the boot cpu is doing while it starts the others, for a waiting
 * secondary to report if the boot cpu stops making progress (a cpu
 * stuck inside a firmware call cannot report anything itself).
 */
static volatile const char *mp_progress = "starting";
static volatile int mp_progress_cpu;
static volatile uint64_t mp_progress_since;

static void
mp_set_progress(const char *what, int cpu)
{
	mp_progress_cpu = cpu;
	mp_progress_since = cpu_cntvct();
	mp_progress = what;
	cpu_mfence();
}
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
/*
 * Each cpu's clock speed, measured once as it starts.  Bootloaders
 * set the clock of the cluster they run on; the Snapdragon 820's
 * other cluster can come out of CPU_ON at its 19.2 MHz reference
 * clock until a cpufreq driver programs its PLL.
 */
static int	arm64_cpu_mhz[MAXCPU];
static int	clocks_changed;	/* re-measure as the cpus are released */
SYSCTL_OPAQUE(_machdep, OID_AUTO, cpu_mhz, CTLFLAG_RD, arm64_cpu_mhz,
    sizeof(arm64_cpu_mhz), "I", "clock speed measured at start, per cpu");

/*
 * Time a chain of dependent adds against the generic timer, with
 * interrupts off.  Every core this port runs on retires one dependent
 * add per cycle, so this is the clock rate to within a few percent.
 */
static int
measure_mhz(void)
{
	const uint64_t iters = 200000;		/* 8 adds each */
	uint64_t t0, t1, n, x = 0, freq = arm64_timer_freq();
	register_t s;
	int pass, best = 0;

	s = intr_disable();
	for (pass = 0; pass < 3; ++pass) {	/* warm up, take the best */
		n = iters;
		__asm __volatile("isb" ::: "memory");
		t0 = cpu_cntvct();
		__asm __volatile(
		    "1:	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	add %0, %0, #1\n"
		    "	subs %1, %1, #1\n"
		    "	b.ne 1b\n"
		    : "+r" (x), "+r" (n) : : "cc");
		__asm __volatile("isb" ::: "memory");
		t1 = cpu_cntvct();
		if (t1 > t0 && freq != 0) {
			int mhz = (int)(iters * 8 * freq / (t1 - t0) / 1000000);
			if (mhz > best)
				best = mhz;
		}
	}
	intr_restore(s);
	return (best);
}

static void
record_mhz(int cpu)
{
	arm64_cpu_mhz[cpu] = measure_mhz();
	kprintf("cpu%d: running at about %d MHz\n", cpu, arm64_cpu_mhz[cpu]);
}

static int
psci_cpu_on(u_long mpidr, int cpu)
{
	vm_paddr_t entry = pmap_kextract((vm_offset_t)mpentry);

	return ((int)psci_call4(PSCI_CPU_ON_64, mpidr, entry, cpu,
	    arm64_bootinfo.bi_psci == 2));
}

static int64_t
psci(uint64_t fn, uint64_t a1, uint64_t a2, uint64_t a3)
{
	return (psci_call4(fn, a1, a2, a3, arm64_bootinfo.bi_psci == 2));
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
	int stall = -1;

	TUNABLE_INT_FETCH("smp.stall", &stall);
	record_mhz(0);
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

	if (ncpus > 1 && bootverbose) {
		int64_t v = psci(PSCI_VERSION, 0, 0, 0);

		kprintf("smp: PSCI %d.%d via %s, starting %d more cpus\n",
		    (int)(v >> 16) & 0xffff, (int)v & 0xffff,
		    arm64_bootinfo.bi_psci == 2 ? "smc" : "hvc", ncpus - 1);
	}

	/*
	 * In a verbose boot every step below is reported, since a cpu
	 * that fails to come up can take the boot cpu down with it
	 * inside the firmware call.
	 */
	for (x = 1; x < ncpus; ++x) {
		mp_set_progress("setting up", x);
		if (bootverbose) {
			kprintf("smp: cpu%d mpidr %lx: power state %d, "
			    "setting up\n", x, mpidr[x],
			    (int)psci(PSCI_AFFINITY_INFO_64, mpidr[x], 0, 0));
		}
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

		if (bootverbose)
			kprintf("smp: cpu%d: CPU_ON\n", x);
		mp_set_progress("inside the PSCI CPU_ON call", x);
		if (stall == x) {
			/*
			 * Test hook (smp.stall=N): freeze here for 7 s, the
			 * way a firmware call that does not return would,
			 * to exercise the waiting cpu's report.
			 */
			uint64_t end = cpu_cntvct() + 7 * arm64_timer_freq();

			while (cpu_cntvct() < end)
				cpu_pause();
		}
		err = psci_cpu_on(mpidr[x], x);
		mp_set_progress("waiting for", x);
		if (bootverbose)
			kprintf("smp: cpu%d: CPU_ON returned %d\n", x, err);
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
			kprintf("cpu%d (mpidr %lx): did not start, power "
			    "state %d\n", x, mpidr[x], (int)psci(
			    PSCI_AFFINITY_INFO_64, mpidr[x], 0, 0));
			break;
		}
		if (bootverbose)
			kprintf("smp: cpu%d: joined after %d ms\n", x, waited);
	}
	mp_set_progress("done starting cpus", x);
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
	if (ncpus > 1) {
		kprintf("SMP: %d cpus\n", ncpus);
		/* Clock the clusters the bootloader left on their XO. */
		if (msm8996_cpu_clocks())
			clocks_changed = 1;
	}
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
	record_mhz(cpu);
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

	/*
	 * Sleep until the boot cpu has started everyone and lets us go
	 * (ap_finish() signals with SEV).  pc64 spins on the MP lock
	 * here instead; on the Snapdragon 820 a busy cpu in one cluster
	 * while the firmware powers up the other was the prime suspect
	 * when bringing up the second cluster hung, and idling the way
	 * Linux's waiting cpus do costs nothing.
	 */
	{
		uint64_t freq = arm64_timer_freq();
		uint64_t next = cpu_cntvct() + 3 * freq;
		uint64_t kctl;

		/*
		 * The timer event stream wakes WFE every 2^16 ticks
		 * (a few ms), so the first secondary can report on the
		 * boot cpu now and then.
		 */
		kctl = READ_SPECIALREG(cntkctl_el1);
		WRITE_SPECIALREG(cntkctl_el1, (kctl & ~0xf0UL) | (15UL << 4) |
		    (1UL << 2));
		for (;;) {
			cpu_lfence();
			if (mp_finish != 0)
				break;
			if (mycpu->gd_cpuid == 1 && cpu_cntvct() > next) {
				kprintf("smp: cpu1 still waiting: boot cpu %s "
				    "cpu%d for %ju s\n", mp_progress,
				    mp_progress_cpu, (uintmax_t)((cpu_cntvct() -
				    mp_progress_since) / freq));
				next = cpu_cntvct() + 3 * freq;
			}
			__asm __volatile("wfe");
		}
		WRITE_SPECIALREG(cntkctl_el1, kctl);
	}
	if (clocks_changed) {
		int before = arm64_cpu_mhz[mycpu->gd_cpuid];

		arm64_cpu_mhz[mycpu->gd_cpuid] = measure_mhz();
		if (arm64_cpu_mhz[mycpu->gd_cpuid] != before) {
			kprintf("cpu%d: now at about %d MHz\n",
			    mycpu->gd_cpuid, arm64_cpu_mhz[mycpu->gd_cpuid]);
		}
	}

	while (try_mplock() == 0) {
		cpu_pause();
		cpu_lfence();
	}
	cpu_invltlb();
	mycpu->gd_other_cpus = smp_startup_mask;
	ATOMIC_CPUMASK_NANDBIT(mycpu->gd_other_cpus, mycpu->gd_cpuid);
	cpu_mfence();
	rel_mplock();

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
	__asm __volatile("dsb sy; sev" : : : "memory");	/* wake ap_init() */
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
