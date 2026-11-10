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

#ifndef _MACHINE_CLOCK_H_
#define	_MACHINE_CLOCK_H_

/*
 * Kernel interface to machine-dependent clock driver.
 */
#ifndef _SYS_TYPES_H_
#include <sys/types.h>
#endif

#ifdef _KERNEL

#ifndef _SYS_SYSTIMER_H_
#include <sys/systimer.h>
#endif

typedef struct TOTALDELAY {
	int		us;
	int		started;
	sysclock_t	last_clock;
} TOTALDELAY;

#endif

/*
 * The generic timer's virtual counter (CNTVCT_EL0) stands in for the
 * TSC.  The architecture guarantees it is monotonic, at least 56 bits
 * wide and synchronized across cpus, so the three "is the TSC usable"
 * flags are constant true.  Machine-independent code that uses the tsc_
 * names (scheduler statistics, lockmgr and token contention timing)
 * works unchanged.
 */
typedef uint64_t tsc_uclock_t;
typedef int64_t	tsc_sclock_t;

#ifdef _KERNEL

extern int	adjkerntz;
extern int	disable_rtc_set;
extern int	tsc_present;
extern int	tsc_invariant;
extern int	tsc_mpsync;
extern int	wall_cmos_clock;
extern tsc_uclock_t tsc_frequency;	/* CNTFRQ_EL0 */
extern tsc_uclock_t tsc_oneus_approx;	/* do not use for fine calc, min 1 */

int	CHECKTIMEOUT(TOTALDELAY *);

#endif /* _KERNEL */

#endif /* !_MACHINE_CLOCK_H_ */
