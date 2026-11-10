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
 * Functions to provide access to special aarch64 instructions.
 */

#ifndef _CPU_CPUFUNC_H_
#define	_CPU_CPUFUNC_H_

#include <sys/cdefs.h>
#ifndef _SYS_TYPES_H_
#include <sys/types.h>
#endif
#include <cpu/armreg.h>

__BEGIN_DECLS

#ifdef	__GNUC__

static __inline void
breakpoint(void)
{
	__asm __volatile("brk #0");
}

/*
 * Spin-wait hint.  YIELD is a NOP on most cores; it is the architected
 * equivalent of x86 PAUSE.
 */
static __inline void
cpu_pause(void)
{
	__asm __volatile("yield");
}

/*
 * Bit scans.  As on x86 the result is undefined for a zero argument.
 */
static __inline u_int
bsfl(u_int mask)
{
	return ((u_int)__builtin_ctz(mask));
}

static __inline u_long
bsfq(u_long mask)
{
	return ((u_long)__builtin_ctzl(mask));
}

static __inline u_long
bsflong(u_long mask)
{
	return ((u_long)__builtin_ctzl(mask));
}

static __inline u_int
bsrl(u_int mask)
{
	return (31U - (u_int)__builtin_clz(mask));
}

static __inline u_long
bsrq(u_long mask)
{
	return (63UL - (u_long)__builtin_clzl(mask));
}

#define	HAVE_INLINE_FFS
static __inline int
ffs(int mask)
{
	return (__builtin_ffs(mask));
}

#define	HAVE_INLINE_FFSL
static __inline int
ffsl(long mask)
{
	return (__builtin_ffsl(mask));
}

#define	HAVE_INLINE_FLS
static __inline int
fls(int mask)
{
	return (mask == 0 ? 0 : 32 - __builtin_clz((u_int)mask));
}

#define	HAVE_INLINE_FLSL
static __inline int
flsl(long mask)
{
	return (mask == 0 ? 0 : 64 - __builtin_clzl((u_long)mask));
}

#define	HAVE_INLINE_FLSLL
static __inline int
flsll(long long mask)
{
	return (flsl((long)mask));
}

/*
 * Interrupt masking.  Only IRQ and FIQ are touched; SError and debug
 * exceptions are left alone.
 */
#ifndef _CPU_DISABLE_INTR_DEFINED
static __inline void
cpu_disable_intr(void)
{
	__asm __volatile("msr daifset, #3");
}
#endif

#ifndef _CPU_ENABLE_INTR_DEFINED
static __inline void
cpu_enable_intr(void)
{
	__asm __volatile("msr daifclr, #3");
}
#endif

static __inline register_t
intr_disable(void)
{
	register_t daif;

	daif = READ_SPECIALREG(daif);
	__asm __volatile("msr daifset, #3");
	return (daif);
}

static __inline void
intr_restore(register_t daif)
{
	WRITE_SPECIALREG(daif, daif);
}

/*
 * Memory barriers.
 *
 * aarch64 is weakly ordered, so unlike x86_64 the load and store fences
 * are real barriers here.  cpu_mfence() orders everything, cpu_lfence()
 * orders loads against later loads/stores, cpu_sfence() orders stores
 * against later stores.
 */
static __inline void
cpu_mfence(void)
{
	__asm __volatile("dmb sy" : : : "memory");
}

static __inline void
cpu_lfence(void)
{
	__asm __volatile("dmb ld" : : : "memory");
}

static __inline void
cpu_sfence(void)
{
	__asm __volatile("dmb st" : : : "memory");
}

/*
 * cpu_ccfence() prevents the compiler from reordering or caching
 * accesses across it.  It emits no instructions.
 */
static __inline void
cpu_ccfence(void)
{
	__asm __volatile("" : : : "memory");
}

static __inline void
cpu_nop(void)
{
	__asm __volatile("nop");
}

/*
 * Wait for an interrupt.  A pending interrupt wakes the cpu even when
 * interrupts are masked, so callers can use the usual "check, then
 * halt with interrupts disabled" idle sequence without a race.
 */
static __inline void
halt(void)
{
	__asm __volatile("wfi");
}

static __inline u_int
loadandclear(volatile u_int *addr)
{
	return (__atomic_exchange_n(addr, 0, __ATOMIC_SEQ_CST));
}

#ifdef _KERNEL

/*
 * Invalidate one page in the current address space on this cpu only.
 */
#ifndef _CPU_INVLPG_DEFINED
static __inline void
cpu_invlpg(void *addr)
{
	__uint64_t va = ((__uint64_t)addr >> 12) & 0xfffffffffffUL;

	__asm __volatile(
	    "tlbi	vaae1, %0\n"
	    "dsb	nsh\n"
	    "isb" : : "r" (va) : "memory");
}
#endif

/*
 * Invalidate the entire TLB on this cpu only.
 */
#ifndef _CPU_INVLTLB_DEFINED
static __inline void
cpu_invltlb(void)
{
	__asm __volatile(
	    "tlbi	vmalle1\n"
	    "dsb	nsh\n"
	    "isb" : : : "memory");
}
#endif

/*
 * The generic timer's virtual counter is the closest thing aarch64
 * has to a TSC: 64 bits, monotonic, and synchronized across cpus.
 */
static __inline __uint64_t
cpu_cntvct(void)
{
	__uint64_t val;

	__asm __volatile("isb\n\tmrs	%0, cntvct_el0" : "=r" (val));
	return (val);
}

#endif /* _KERNEL */

#endif	/* __GNUC__ */

__END_DECLS

#endif /* !_CPU_CPUFUNC_H_ */
