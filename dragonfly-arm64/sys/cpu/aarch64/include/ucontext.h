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

#ifndef _CPU_UCONTEXT_H_
#define	_CPU_UCONTEXT_H_

#include <sys/cdefs.h>
#include <machine/stdint.h>

struct __mcontext {
	/*
	 * The fields from mc_sp through mc_x[] must match the layout of
	 * struct trapframe; sendsig and sigreturn bcopy between the two.
	 * The whole structure must also match struct sigcontext after
	 * its leading sigset_t.
	 */
	__register_t	mc_onstack;	/* XXX - sigcontext compat. */
	__register_t	mc_sp;
	__register_t	mc_lr;
	__register_t	mc_elr;
	__register_t	mc_spsr;
	__register_t	mc_esr;
	__register_t	mc_far;
	__register_t	mc_x[30];

	unsigned int	mc_len;		/* sizeof(mcontext_t) */
	unsigned int	mc_fpformat;
	unsigned int	mc_ownedfp;
	unsigned int	mc_reserved;
	__register_t	mc_tpidr;	/* userland thread pointer */
	__register_t	mc_unused[5];

	/* 16 byte aligned: FP/SIMD state, see struct savefpu */
	__uint128_t	mc_fpregs[33];
} __aligned(16);

typedef struct __mcontext mcontext_t;

#define _MC_FPFMT_NODEV		0x10000 /* device not present or configured */
#define _MC_FPFMT_VFP		0x10001 /* FP/AdvSIMD, struct savefpu */

#define _MC_FPOWNED_NONE	0x20000 /* FP state not used */
#define _MC_FPOWNED_FPU		0x20001 /* FP state came from FPU */
#define _MC_FPOWNED_PCB		0x20002 /* FP state came from PCB */

#endif /* !_CPU_UCONTEXT_H_ */
