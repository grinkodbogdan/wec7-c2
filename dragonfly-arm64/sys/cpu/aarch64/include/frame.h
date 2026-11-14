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

#ifndef _CPU_FRAME_H_
#define _CPU_FRAME_H_

/*
 * Exception/Trap Stack Frame.
 *
 * Built by the vector code in exception.S; the TF_* offsets there must
 * be kept in sync with this structure (genassym will take that over).
 * The structure is 288 bytes, a multiple of the 16 byte stack alignment.
 */
struct trapframe {
	__uint64_t	tf_sp;		/* sp at the time of the exception */
	__uint64_t	tf_lr;		/* x30 */
	__uint64_t	tf_elr;		/* exception return address (pc) */
	__uint64_t	tf_spsr;	/* saved processor state */
	__uint32_t	tf_esr;		/* exception syndrome (ESR_EL1) */
	__uint32_t	tf_trapno;	/* software trap type, <cpu/trap.h> */
	__uint64_t	tf_far;		/* fault address */
	__uint64_t	tf_x[30];	/* x0 - x29 */
};

/*
 * Interrupt frame.  The layout is identical to struct trapframe (the
 * vector code builds one frame type for everything) but it is a
 * distinct type, as it is on x86_64, because machine-
 * independent code declares interrupt and systimer callbacks in terms
 * of it.
 */
struct intrframe {
	__uint64_t	if_sp;
	__uint64_t	if_lr;
	__uint64_t	if_elr;
	__uint64_t	if_spsr;
	__uint32_t	if_esr;
	__uint32_t	if_vec;		/* interrupt number, set by dispatch */
	__uint64_t	if_far;
	__uint64_t	if_x[30];
};

#define	TF_FP(tf)		((tf)->tf_x[29])

/* CLKF_* take a struct intrframe, TRAPF_* a struct trapframe. */
#define	CLKF_USERMODE(framep)	(((framep)->if_spsr & PSR_M_MASK) == PSR_M_EL0t)
#define	CLKF_INTR(intr_nest)	((intr_nest) > 1)
#define	CLKF_PC(framep)		((framep)->if_elr)
#define	TRAPF_USERMODE(framep)	(((framep)->tf_spsr & PSR_M_MASK) == PSR_M_EL0t)
#define	TRAPF_PC(framep)	((framep)->tf_elr)

#define	INTR_TO_TRAPFRAME(frame) ((struct trapframe *)(frame))

#endif /* _CPU_FRAME_H_ */
