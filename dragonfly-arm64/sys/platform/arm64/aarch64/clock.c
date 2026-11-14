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
 * ARM generic timer.
 *
 * The virtual counter (CNTVCT_EL0) is the system's free-running
 * timebase, registered as a cputimer; it takes the role the TSC or
 * HPET has on pc64.  The virtual timer comparator (CNTV_TVAL_EL0)
 * is the one-shot interrupt source systimers run off, registered as a
 * cputimer_intr; it takes the role of the local APIC timer.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/systimer.h>
#include <sys/time.h>
#include <sys/globaldata.h>
#include <sys/sysctl.h>

#include <sys/thread2.h>

#include <machine/clock.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>

int	disable_rtc_set;
int	adjkerntz;
int	wall_cmos_clock;
int	tsc_present = 1;
int	tsc_invariant = 1;
int	tsc_mpsync = 1;
tsc_uclock_t tsc_frequency;
tsc_uclock_t tsc_oneus_approx = 1;

static sysclock_t arm_cputimer_count(void);
static void	arm_cputimer_construct(struct cputimer *, sysclock_t);
static void	arm_timer_intr_reload(struct cputimer_intr *, sysclock_t);
static void	arm_timer_intr_enable(struct cputimer_intr *);

static struct cputimer arm_cputimer = {
	.next		= SLIST_ENTRY_INITIALIZER,
	.name		= "generic-timer",
	.pri		= CPUTIMER_PRI_TSC,
	.type		= CPUTIMER_TSC,
	.count		= arm_cputimer_count,
	.fromhz		= cputimer_default_fromhz,
	.fromus		= cputimer_default_fromus,
	.construct	= arm_cputimer_construct,
	.destruct	= cputimer_default_destruct,
	.freq		= 0
};

static struct cputimer_intr arm_cputimer_intr = {
	.freq		= 0,
	.reload		= arm_timer_intr_reload,
	.enable		= arm_timer_intr_enable,
	.config		= cputimer_intr_default_config,
	.restart	= cputimer_intr_default_restart,
	.pmfixup	= cputimer_intr_default_pmfixup,
	.initclock	= cputimer_intr_default_initclock,
	.pcpuhand	= NULL,
	.next		= SLIST_ENTRY_INITIALIZER,
	.name		= "generic-timer",
	.type		= CPUTIMER_INTR_LAPIC,
	.prio		= CPUTIMER_INTR_PRIO_LAPIC,
	.caps		= CPUTIMER_INTR_CAP_NONE,
	.priv		= NULL
};

/*
 * Frequency of the generic timer's counter.  The firmware is supposed
 * to program it into CNTFRQ_EL0; where it does not, the device tree's
 * timer node carries it instead.
 */
uint64_t
arm64_timer_freq(void)
{
	uint64_t freq = READ_SPECIALREG(cntfrq_el0);

	if (freq == 0)
		freq = arm64_bootinfo.bi_timer_freq;
	if (freq == 0)
		freq = 19200000;	/* better than dividing by zero */
	return (freq);
}

static sysclock_t
arm_cputimer_count(void)
{
	return (cpu_cntvct() + arm_cputimer.base);
}

static void
arm_cputimer_construct(struct cputimer *timer, sysclock_t oclock)
{
	timer->base = 0;
	timer->base = oclock - arm_cputimer_count();
}

/*
 * Arrange for a timer interrupt in (reload) ticks of the system
 * cputimer.  A request never pushes an already-pending earlier
 * interrupt further out.
 */
static void
arm_timer_intr_reload(struct cputimer_intr *cti, sysclock_t reload)
{
	struct globaldata *gd = mycpu;

	if ((ssysclock_t)reload < 0)
		reload = 1;
	reload = muldivu64(reload, cti->freq, sys_cputimer->freq);
	if (reload < 2)
		reload = 2;
	if (reload > 0x7fffffff)
		reload = 0x7fffffff;

	if (gd->gd_timer_running) {
		int32_t remaining = (int32_t)READ_SPECIALREG(cntv_tval_el0);

		if (remaining > 0 && reload >= (sysclock_t)remaining)
			return;
	}
	gd->gd_timer_running = 1;
	WRITE_SPECIALREG(cntv_tval_el0, reload);
	WRITE_SPECIALREG(cntv_ctl_el0, CNTx_CTL_ENABLE);
}

static void
arm_timer_intr_enable(struct cputimer_intr *cti __unused)
{
	gic_enable_irq(arm64_bootinfo.bi_timer_irq);
}

/*
 * Keep the timer from asserting its interrupt line; see arm64_intr().
 */
void
clock_intr_quiesce(void)
{
	WRITE_SPECIALREG(cntv_ctl_el0, CNTx_CTL_ENABLE | CNTx_CTL_IMASK);
}

/*
 * The timer fired.  (frame) is NULL when the interrupt was deferred
 * and is being delivered from splz().
 */
void
clock_intr(struct intrframe *frame)
{
	struct globaldata *gd = mycpu;
	sysclock_t count;

	gd->gd_timer_running = 0;
	count = sys_cputimer->count();
	++gd->gd_cnt.v_timer;
	systimer_intr(&count, 0, frame);
}

static void
cpu_initclocks(void *arg __unused)
{
	uint64_t freq = arm64_timer_freq();

	tsc_frequency = freq;
	tsc_oneus_approx = freq / 1000000 ? freq / 1000000 : 1;

	/* The interrupt source first: selecting a cputimer reloads it. */
	arm_cputimer_intr.freq = freq;
	cputimer_intr_register(&arm_cputimer_intr);
	cputimer_intr_select(&arm_cputimer_intr, 0);

	arm_cputimer.freq = freq;
	cputimer_register(&arm_cputimer);
	cputimer_select(&arm_cputimer, 0);
}
SYSINIT(clocksarm, SI_BOOT2_CLOCKREG, SI_ORDER_FIRST, cpu_initclocks, NULL);

/*
 * Wait at least (n) microseconds.  Usable from the very start of
 * boot: the counter needs no setup.
 */
void
DELAY(int n)
{
	uint64_t freq = arm64_timer_freq();
	uint64_t end;

	end = cpu_cntvct() + (uint64_t)n * freq / 1000000 + 1;
	while ((int64_t)(end - cpu_cntvct()) > 0)
		cpu_pause();
}

int
CHECKTIMEOUT(TOTALDELAY *tdd)
{
	sysclock_t delta;
	int us;

	if (tdd->started == 0) {
		tdd->last_clock = sys_cputimer->count();
		tdd->started = 1;
		return (0);
	}
	delta = sys_cputimer->count() - tdd->last_clock;
	us = (u_int64_t)delta * (u_int64_t)1000000 /
	    (u_int64_t)sys_cputimer->freq;
	tdd->last_clock += (u_int64_t)us * (u_int64_t)sys_cputimer->freq /
	    1000000;
	tdd->us -= us;
	return (tdd->us < 0);
}

/*
 * Initialize the time of day.  There is no battery backed clock
 * driver yet, so the only source is the time stamp the caller got
 * from the root filesystem.
 */
void
inittodr(time_t base)
{
	struct timespec ts;

	if (base) {
		ts.tv_sec = base;
		ts.tv_nsec = 0;
		set_timeofday(&ts);
	}
}

void
resettodr(void)
{
}
