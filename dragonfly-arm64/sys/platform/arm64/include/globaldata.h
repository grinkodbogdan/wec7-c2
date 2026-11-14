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

#ifndef _MACHINE_GLOBALDATA_H_
#define _MACHINE_GLOBALDATA_H_

#if defined(_KERNEL) || defined(_KERNEL_STRUCTURES)

#ifndef _SYS_GLOBALDATA_H_
#include <sys/globaldata.h>	/* struct globaldata */
#endif
#ifndef _SYS_THREAD_H_
#include <sys/thread.h>		/* struct thread */
#endif
#include <cpu/fpu.h>

/*
 * Note on interrupt control.  Pending interrupts not yet dispatched are
 * marked in gd_ipending/gd_spending.  Once dispatched the interrupt's
 * pending bit is cleared and the interrupt is masked.  Upon completion
 * the interrupt is unmasked.
 *
 * For edge triggered interrupts interrupts may be enabled again at this
 * point and if they occur before the interrupt service routine is
 * complete the service routine will loop.
 *
 * The current thread's cpl is stored in the thread structure.
 */
struct mdglobaldata {
	struct globaldata mi;
	struct thread   *gd_fputhread;	/* owner of the live FP state */
	int		gd_spending;	/* software interrupt pending */
	int		gd_sdelayed;	/* delayed software ints */
	u_long		gd_mpidr;	/* this cpu's MPIDR_EL1 affinity */
	register_t	gd_scratch[2];	/* exception entry scratch */
	u_long		gd_ipending[3];	/* hardware interrupts pending */
};

#define MDGLOBALDATA_BASEALLOC_SIZE	\
	((sizeof(struct mdglobaldata) + PAGE_MASK) & ~PAGE_MASK)
#define MDGLOBALDATA_BASEALLOC_PAGES	\
	(MDGLOBALDATA_BASEALLOC_SIZE / PAGE_SIZE)
#define MDGLOBALDATA_PAD		\
	(MDGLOBALDATA_BASEALLOC_SIZE - sizeof(struct mdglobaldata))

/*
 * Per-cpu private data.  Unlike x86_64 there is no TSS, GDT or
 * trampoline frame to carry; each cpu needs only its globaldata and an
 * idle thread stack.
 */
struct privatespace {
	/* main data page */
	struct mdglobaldata mdglobaldata;
	char		__filler0[MDGLOBALDATA_PAD];

	/* idle stack (UPAGES pages) */
	char		idlestack[UPAGES * PAGE_SIZE];
} __packed;

#define mdcpu  		((struct mdglobaldata *)_get_mycpu())

#endif

#ifdef _KERNEL

extern struct privatespace *CPU_prvspace[];

#endif

#endif
