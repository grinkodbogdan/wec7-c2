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

#ifndef _CPU_SIGNAL_H_
#define	_CPU_SIGNAL_H_

#include <sys/cdefs.h>

/*
 * Machine-dependent signal definitions
 */

typedef int sig_atomic_t;

#if __BSD_VISIBLE

#include <sys/_sigset.h>
#include <machine/trap.h>	/* codes for SIGILL, SIGFPE */

/*
 * Information pushed on stack when a signal is delivered.  This is used
 * by the kernel to restore state following execution of the signal
 * handler.  It is also made available to the handler to allow it to
 * restore state properly if a non-standard exit is performed.
 *
 * The sequence of the fields from sc_onstack on must match those in
 * mcontext_t, which in turn starts with the layout of struct trapframe
 * so sendsig can bcopy between them.
 */
struct	sigcontext {
	sigset_t	sc_mask;	/* signal mask to restore */

	long		sc_onstack;	/* sigstack state to restore */
	long		sc_sp;
	long		sc_lr;
	long		sc_elr;
	long		sc_spsr;
	long		sc_esr;
	long		sc_far;
	long		sc_x[30];

	unsigned int	sc_len;
	unsigned int	sc_fpformat;
	unsigned int	sc_ownedfp;
	unsigned int	sc_reserved;
	long		sc_tpidr;	/* userland thread pointer */
	long		sc_unused[5];

	/* 16 byte aligned: FP/SIMD state, see struct savefpu */
	__uint128_t	sc_fpregs[33];
} __aligned(16);

#endif /* __BSD_VISIBLE */

#if defined(_KERNEL) || defined(_KERNEL_STRUCTURES)
/*
 * The sigcode is copied into a userland-visible page.
 */
#define SZSIGCODE_EXTRA_BYTES	(1*PAGE_SIZE)
#endif

#endif /* !_CPU_SIGNAL_H_ */
