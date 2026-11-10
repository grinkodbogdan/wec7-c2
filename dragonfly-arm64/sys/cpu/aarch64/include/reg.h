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

#ifndef _CPU_REG_H_
#define	_CPU_REG_H_

/*
 * Register set accessible via /proc/$pid/regs and PT_{SET,GET}REGS.
 */
struct reg {
	register_t	r_x[30];
	register_t	r_lr;
	register_t	r_sp;
	register_t	r_elr;
	register_t	r_spsr;
};

/*
 * Register set accessible via /proc/$pid/fpregs.
 */
struct fpreg {
	__uint128_t	fp_q[32];
	__uint32_t	fp_sr;
	__uint32_t	fp_cr;
};

/*
 * Register set accessible via /proc/$pid/dbregs.
 */
struct dbreg {
	__uint64_t	db_bvr[16];	/* breakpoint value registers */
	__uint32_t	db_bcr[16];	/* breakpoint control registers */
	__uint64_t	db_wvr[16];	/* watchpoint value registers */
	__uint32_t	db_wcr[16];	/* watchpoint control registers */
};

#endif /* !_CPU_REG_H_ */
