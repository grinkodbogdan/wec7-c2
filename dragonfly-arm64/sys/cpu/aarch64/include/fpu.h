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

#ifndef _CPU_FPU_H_
#define	_CPU_FPU_H_

#include <machine/stdint.h>

/*
 * FP/Advanced SIMD register state: 32 128-bit vector registers plus
 * the status and control registers.  This is the aarch64 counterpart
 * of x86_64's union savefpu (<cpu/npx.h>).
 *
 * SVE state is not saved; SVE stays disabled for userland until this
 * grows a variable-length save area.
 */
struct savefpu {
	__uint128_t	sv_q[32];
	__uint32_t	sv_fpsr;
	__uint32_t	sv_fpcr;
} __aligned(16);

#ifdef _KERNEL
struct thread;

void	fpu_save(struct savefpu *);
void	fpu_restore(const struct savefpu *);
void	fpu_heavy_switch(struct thread *);
#endif

#endif /* !_CPU_FPU_H_ */
