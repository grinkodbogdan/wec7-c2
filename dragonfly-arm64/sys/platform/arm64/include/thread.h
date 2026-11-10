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

#ifndef	_MACHINE_THREAD_H_
#define	_MACHINE_THREAD_H_

#include <cpu/fpu.h>
#include <cpu/vframe.h>

struct md_thread {
    unsigned int	mtd_cpl;
    struct savefpu	*mtd_savefpu;
    struct savetls	mtd_savetls;
};

#ifdef _KERNEL

#define td_cpl		td_mach.mtd_cpl
#define td_tls		td_mach.mtd_savetls
#define td_savefpu      td_mach.mtd_savefpu

/*
 * mycpu() retrieves the base of the current cpu's globaldata structure.
 *
 * The pointer lives in TPIDR_EL1, a register reserved for exactly this
 * kind of OS use; it plays the role %gs plays on x86_64.  Reading a
 * system register is not something the compiler can cache across a
 * call, so no dummy memory reference is needed to defeat that.
 *
 * In DragonFly a thread can be preempted, but it cannot move to another
 * cpu preemptively so the 'gd' pointer is good until you block.
 */

struct globaldata;

static __inline
struct globaldata *
_get_mycpu(void)
{
    struct globaldata *gd;

    __asm __volatile("mrs %0, tpidr_el1" : "=r" (gd));
    return(gd);
}

#define mycpu	_get_mycpu()
#define mycpuid (_get_mycpu()->gd_cpuid)

/*
 * note: curthread is never NULL, but curproc can be.  Also note that
 * in DragonFly, the current pcb is stored in the thread structure.
 */
#define curthread	mycpu->gd_curthread
#define	curproc		curthread->td_proc

#endif	/* _KERNEL */

#endif	/* !_MACHINE_THREAD_H_ */
