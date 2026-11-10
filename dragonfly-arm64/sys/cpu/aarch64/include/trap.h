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

#ifndef _CPU_TRAP_H_
#define	_CPU_TRAP_H_

/*
 * Trap type values.
 *
 * aarch64 reports the reason for an exception in ESR_ELx (see
 * <cpu/armreg.h>) rather than through a vector number, so these are
 * software values: the trap code maps exception classes onto them for
 * the benefit of machine-independent consumers (signal codes, ddb,
 * ktrace) that were written against the x86 numbering.
 */
#define	T_PRIVINFLT	1	/* privileged instruction */
#define	T_BPTFLT	3	/* breakpoint instruction */
#define	T_ARITHTRAP	6	/* arithmetic trap */
#define T_ASTFLT	7	/* system forced exception */
#define	T_PROTFLT	9	/* protection fault */
#define	T_TRCTRAP	10	/* debug exception (sic) */
#define	T_PAGEFLT	12	/* page fault */
#define	T_ALIGNFLT	14	/* alignment fault */
#define	T_DIVIDE	18	/* integer divide fault */
#define	T_NMI		19	/* non-maskable trap */
#define	T_DNA		22	/* device (fpu) not available fault */
#define	T_MCHK		28	/* machine check (SError) */
#define	T_RESERVED	256	/* reserved to reserved+255 (unknown) */

/* definitions for <sys/signal.h> */
#define	    ILL_PRIVIN_FAULT	T_PRIVINFLT
#define	    ILL_ALIGN_FAULT	T_ALIGNFLT

/* codes for SIGFPE/ARITHTRAP */
#define	FPE_INTOVF_TRAP	0x1	/* integer overflow */
#define	FPE_INTDIV_TRAP	0x2	/* integer divide by zero */
#define	FPE_FLTDIV_TRAP	0x3	/* floating/decimal divide by zero */
#define	FPE_FLTOVF_TRAP	0x4	/* floating overflow */
#define	FPE_FLTUND_TRAP	0x5	/* floating underflow */
#define	FPE_FPU_NP_TRAP	0x6	/* floating point unit not present  */
#define	FPE_SUBRNG_TRAP	0x7	/* subrange out of bounds */

/* codes for SIGBUS */
#define	    BUS_PAGE_FAULT	T_PAGEFLT	/* page fault protection base */

/* Trap's coming from user mode */
#define	T_USER	0x100

#endif /* !_CPU_TRAP_H_ */
