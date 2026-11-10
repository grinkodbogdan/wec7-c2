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
 * ARM generic timer, virtual timer channel.
 *
 * Bring-up version: a fixed periodic tick on the boot cpu.  The real
 * port wraps CNTVCT in a struct cputimer (free-running timebase, see
 * kern/kern_cputimer.c) and CNTV_TVAL in a struct cputimer_intr
 * (one-shot event source), which is what DragonFly's systimer code
 * drives; that replaces both the i8254/TSC and LAPIC timer roles.
 */

#include <sys/types.h>
#include <cpu/armreg.h>
#include <cpu/cpufunc.h>
#include <machine/md_var.h>

volatile u_long	arm64_ticks;
static uint64_t	timer_reload;

void
cpu_initclocks_early(int irq, int hz)
{
	timer_reload = READ_SPECIALREG(cntfrq_el0) / hz;

	gic_enable_irq(irq);
	WRITE_SPECIALREG(cntv_tval_el0, timer_reload);
	WRITE_SPECIALREG(cntv_ctl_el0, CNTx_CTL_ENABLE);
}

void
clock_intr(void)
{
	/* Writing TVAL re-arms the comparator and deasserts the line. */
	WRITE_SPECIALREG(cntv_tval_el0, timer_reload);
	++arm64_ticks;
}
