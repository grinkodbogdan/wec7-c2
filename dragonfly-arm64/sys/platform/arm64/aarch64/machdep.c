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
 * arm64 platform early machine-dependent startup.
 *
 * initarm() is the C entry point reached from locore.S with the MMU
 * on and the kernel running at KERNBASE.  On pc64 the equivalent is
 * hammer_time(), which ends by returning to locore and calling
 * mi_startup().  This version performs the hardware bring-up that
 * hammer_time() depends on and then exercises it with self-tests; the
 * machine-independent kernel is not linked in yet, so it powers the
 * machine off instead of entering mi_startup().
 */

#include <sys/types.h>
#include <sys/_null.h>
#include <sys/copyright.h>
#include <cpu/armreg.h>
#include <cpu/atomic.h>
#include <cpu/cpufunc.h>
#include <cpu/param.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	EARLY_HZ		100

#define	PSCI_SYSTEM_OFF		0x84000008UL

/* Fallback when the device tree names no UART: QEMU virt's PL011. */
#define	FALLBACK_UART		0x09000000UL

int	kprintf(const char *fmt, ...) __printflike(1, 2);

extern char kernbase[], etext[], _end[];

struct arm64_bootinfo arm64_bootinfo;

static int selftest_failures;

#define	CHECK(expr)							\
do {									\
	if (!(expr)) {							\
		kprintf("    FAILED: %s\n", #expr);			\
		++selftest_failures;					\
	}								\
} while (0)

void
cpu_poweroff(void)
{
	cpu_disable_intr();
	if (arm64_bootinfo.bi_psci != 0)
		psci_call(PSCI_SYSTEM_OFF, 0, 0, arm64_bootinfo.bi_psci == 2);
	for (;;)
		halt();
}

/*
 * Run the primitives in <cpu/atomic.h> and <cpu/cpufunc.h> on the real
 * (emulated) hardware.  Machine-independent code depends on the exact
 * return value conventions here, and they are easy to get subtly wrong.
 */
static void
selftest_atomic(void)
{
	volatile u_int i = 5;
	volatile u_long l = 0x100000000UL;
	u_int iold;
	u_long lold;
	__atomic_intr_t ai;

	atomic_add_int(&i, 3);
	CHECK(i == 8);
	atomic_subtract_int(&i, 1);
	CHECK(i == 7);
	atomic_set_int(&i, 0x10);
	CHECK(i == 0x17);
	atomic_clear_int(&i, 0x3);
	CHECK(i == 0x14);
	CHECK(atomic_fetchadd_int(&i, 4) == 0x14 && i == 0x18);
	CHECK(atomic_cmpset_int(&i, 0x18, 1) != 0 && i == 1);
	CHECK(atomic_cmpset_int(&i, 0x18, 2) == 0 && i == 1);
	iold = 9;
	CHECK(atomic_fcmpset_int(&i, &iold, 2) == 0 && iold == 1);
	CHECK(atomic_fcmpset_int(&i, &iold, 2) != 0 && i == 2);
	CHECK(atomic_swap_int((volatile int *)&i, 40) == 2 && i == 40);
	CHECK(atomic_readandclear_int(&i) == 40 && i == 0);
	CHECK(atomic_testandset_int(&i, 3) == 0 && i == 8);
	CHECK(atomic_testandset_int(&i, 3) != 0);
	CHECK(atomic_testandclear_int(&i, 3) != 0 && i == 0);
	CHECK(atomic_poll_acquire_int(&i) != 0);
	CHECK(atomic_poll_acquire_int(&i) == 0);
	atomic_poll_release_int(&i);
	CHECK(i == 0);

	atomic_add_long(&l, 0x100000000UL);
	CHECK(l == 0x200000000UL);
	CHECK(atomic_cmpset_long(&l, 0x200000000UL, 0xffffffffffUL) != 0);
	lold = 0;
	CHECK(atomic_fcmpset_long(&l, &lold, 1) == 0 &&
	    lold == 0xffffffffffUL);
	CHECK(atomic_fetchadd_long(&l, 1) == 0xffffffffffUL &&
	    l == 0x10000000000UL);
	CHECK(atomic_testandset_long(&l, 63) == 0 &&
	    l == 0x8000010000000000UL);
	CHECK(atomic_load_acq_long(&l) == 0x8000010000000000UL);
	atomic_store_rel_long(&l, 0);
	CHECK(l == 0);

	atomic_intr_init(&ai);
	CHECK(atomic_intr_cond_try(&ai) == 0);
	CHECK(atomic_intr_cond_try(&ai) == 1);
	CHECK(atomic_intr_cond_test(&ai) != 0);
	CHECK(atomic_intr_handler_disable(&ai) != 0);
	CHECK(atomic_intr_handler_is_enabled(&ai) != 0);
	atomic_intr_handler_enable(&ai);
	CHECK(atomic_intr_handler_is_enabled(&ai) == 0);
	atomic_intr_cond_exit(&ai, NULL, NULL);
	CHECK(ai == 0);

	CHECK(bsfl(0x80) == 7 && bsrl(0x80) == 7);
	CHECK(bsfq(0x8000000000UL) == 39 && bsrq(0x8000000001UL) == 39);
	CHECK(ffs(0) == 0 && ffs(0x18) == 4);
	CHECK(fls(0) == 0 && fls(0x18) == 5 && fls(-1) == 32);
	CHECK(ffsl(0x100000000L) == 33 && flsl(0x100000000L) == 33);
	CHECK(__bswap32(0x11223344) == 0x44332211);
	CHECK(__bswap16(0x1122) == 0x2211);
	CHECK(__bswap64(0x1122334455667788UL) == 0x8877665544332211UL);
}

static void
selftest_traps(void)
{
	u_long val = 0;

	/* Synchronous exception entry, frame save/restore and return. */
	breakpoint();

	/* A mapped address reads back without faulting... */
	CHECK(probe_read64((vm_offset_t)kernbase, &val) == 0 && val != 0);

	/*
	 * ...and an unmapped one takes a data abort that the onfault
	 * hook recovers from.  With TTBR0 walks disabled every low
	 * address is unmapped, as is the hole below KERNBASE.
	 */
	CHECK(probe_read64(0x1000, &val) != 0);
	CHECK(probe_read64(KERNBASE - PAGE_SIZE, &val) != 0);
}

static void
selftest_clock(void)
{
	uint64_t freq = READ_SPECIALREG(cntfrq_el0);
	uint64_t start, delta;
	u_long ms;

	cpu_initclocks_early(arm64_bootinfo.bi_timer_irq, EARLY_HZ);
	start = cpu_cntvct();
	cpu_enable_intr();
	while (arm64_ticks < EARLY_HZ / 2)
		halt();
	cpu_disable_intr();
	delta = cpu_cntvct() - start;
	ms = delta * 1000 / freq;
	kprintf("  %lu ticks at %d Hz took %lu ms by the counter\n",
	    arm64_ticks, EARLY_HZ, ms);
	CHECK(ms >= 480 && ms <= 560);
}

void
initarm(vm_paddr_t fdt_pa, vm_paddr_t kernphys)
{
	struct arm64_bootinfo *bi = &arm64_bootinfo;
	vm_size_t physmem = 0;
	int fdt_ok;
	int nirqs;
	int i;

	/*
	 * The device tree lives in RAM somewhere above the kernel; reach
	 * it through the bootstrap direct map.
	 */
	bi->bi_timer_irq = 27;
	fdt_ok = (fdt_scan((void *)PHYS_TO_DMAP(fdt_pa), bi) == 0);
	cninit_early(bi->bi_uart_base ? bi->bi_uart_base : FALLBACK_UART);

	kprintf("\n%s", copyright);
	kprintf("DragonFly/arm64 platform bring-up kernel (MI kernel not "
	    "linked)\n\n");

	if (!fdt_ok)
		kprintf("WARNING: no valid device tree at 0x%lx\n", fdt_pa);
	kprintf("Machine: %s (%s)\n",
	    bi->bi_model ? bi->bi_model : "unknown",
	    bi->bi_compatible ? bi->bi_compatible : "unknown");
	kprintf("FDT at 0x%lx (%u bytes), bootargs \"%s\"\n", fdt_pa,
	    bi->bi_fdt_size, bi->bi_bootargs ? bi->bi_bootargs : "");
	kprintf("Console: PL011 at 0x%lx (stdout-path %s)\n",
	    bi->bi_uart_base, bi->bi_stdout ? bi->bi_stdout : "unset");

	identify_cpu();
	kprintf("  %d cpu(s) in device tree, boot cpu only\n", bi->bi_ncpus);

	for (i = 0; i < bi->bi_nmem; ++i) {
		kprintf("Physical memory: 0x%016lx - 0x%016lx (%lu MB)\n",
		    bi->bi_mem[i].mr_start,
		    bi->bi_mem[i].mr_start + bi->bi_mem[i].mr_size - 1,
		    bi->bi_mem[i].mr_size >> 20);
		physmem += bi->bi_mem[i].mr_size;
	}
	kprintf("real memory = %lu (%lu MB)\n", physmem, physmem >> 20);
	kprintf("Kernel: physical 0x%lx, virtual %p - %p (text ends %p)\n",
	    kernphys, kernbase, _end, etext);

	/*
	 * The kernel now runs entirely in the upper half.  Stop walking
	 * TTBR0 so the lower half is free for userland and stray NULL
	 * dereferences fault.
	 */
	WRITE_SPECIALREG(tcr_el1, READ_SPECIALREG(tcr_el1) | TCR_EPD0);
	cpu_invltlb();
	kprintf("MMU: TTBR0 identity map retired, DMAP at 0x%lx\n",
	    DMAP_MIN_ADDRESS);

	kprintf("\nSelf-test: atomic ops and bit primitives\n");
	selftest_atomic();
	kprintf("Self-test: exception entry and fault recovery\n");
	selftest_traps();

	if (bi->bi_gic_version == 2 && bi->bi_gicc_base != 0) {
		nirqs = gic_init(bi->bi_gicd_base, bi->bi_gicc_base);
		kprintf("GICv2: distributor 0x%lx, cpu interface 0x%lx, "
		    "%d irqs\n", bi->bi_gicd_base, bi->bi_gicc_base, nirqs);
		kprintf("Self-test: generic timer interrupt (irq %d)\n",
		    bi->bi_timer_irq);
		selftest_clock();
	} else {
		kprintf("Interrupt controller: GIC version %d not supported, "
		    "skipping timer test\n", bi->bi_gic_version);
	}

	if (selftest_failures == 0)
		kprintf("\nAll self-tests passed.\n");
	else
		kprintf("\n%d self-test check(s) FAILED.\n", selftest_failures);
	kprintf("Next milestone: pmap bootstrap and mi_startup().  "
	    "Powering off.\n");
	cpu_poweroff();
}
