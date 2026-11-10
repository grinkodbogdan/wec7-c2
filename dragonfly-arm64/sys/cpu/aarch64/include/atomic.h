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

#ifndef _CPU_ATOMIC_H_
#define _CPU_ATOMIC_H_

#ifndef _SYS_TYPES_H_
#include <sys/types.h>
#endif

/*
 * Atomic operations for aarch64.
 *
 * Everything is expressed with the compiler's __atomic builtins, which
 * expand to LL/SC (LDXR/STXR) loops on ARMv8.0 and to single LSE
 * instructions (CAS, SWP, LDADD, ...) when built with -march=armv8.1-a
 * or later.  All operations are sequentially consistent, which matches
 * the strong ordering x86_64's locked instructions give callers and is
 * what the machine-independent kernel was written against.  Relaxing
 * individual operations to acquire/release ordering is a later
 * optimization, not a correctness requirement.
 *
 * The _xacquire/_xrelease (Intel TSX lock elision) variants have no
 * aarch64 equivalent and are plain aliases.
 */

#define	ATOMIC_OPS(NAME, TYPE, SFX)					\
static __inline void							\
atomic_set_##NAME(volatile TYPE *p, TYPE v)				\
{									\
	(void)__atomic_fetch_or(p, v, __ATOMIC_SEQ_CST);		\
}									\
static __inline void							\
atomic_clear_##NAME(volatile TYPE *p, TYPE v)				\
{									\
	(void)__atomic_fetch_and(p, ~v, __ATOMIC_SEQ_CST);		\
}									\
static __inline void							\
atomic_add_##NAME(volatile TYPE *p, TYPE v)				\
{									\
	(void)__atomic_fetch_add(p, v, __ATOMIC_SEQ_CST);		\
}									\
static __inline void							\
atomic_subtract_##NAME(volatile TYPE *p, TYPE v)			\
{									\
	(void)__atomic_fetch_sub(p, v, __ATOMIC_SEQ_CST);		\
}									\
/* Non-locked variants: only atomic with respect to interrupts on x86. */\
static __inline void							\
atomic_set_##NAME##_nonlocked(volatile TYPE *p, TYPE v)			\
{									\
	(void)__atomic_fetch_or(p, v, __ATOMIC_SEQ_CST);		\
}									\
static __inline void							\
atomic_clear_##NAME##_nonlocked(volatile TYPE *p, TYPE v)		\
{									\
	(void)__atomic_fetch_and(p, ~v, __ATOMIC_SEQ_CST);		\
}									\
static __inline void							\
atomic_add_##NAME##_nonlocked(volatile TYPE *p, TYPE v)			\
{									\
	(void)__atomic_fetch_add(p, v, __ATOMIC_SEQ_CST);		\
}									\
static __inline void							\
atomic_subtract_##NAME##_nonlocked(volatile TYPE *p, TYPE v)		\
{									\
	(void)__atomic_fetch_sub(p, v, __ATOMIC_SEQ_CST);		\
}									\
/*									\
 * cmpset: if (*dst == old) *dst = new.  Returns non-zero on success.	\
 */									\
static __inline int							\
atomic_cmpset_##NAME(volatile TYPE *dst, TYPE old, TYPE new)		\
{									\
	return (__atomic_compare_exchange_n(dst, &old, new, 0,		\
	    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));			\
}									\
/*									\
 * fcmpset: as cmpset, but on failure *old is updated with the value	\
 * that was found.							\
 */									\
static __inline int							\
atomic_fcmpset_##NAME(volatile TYPE *dst, TYPE *old, TYPE new)		\
{									\
	return (__atomic_compare_exchange_n(dst, old, new, 0,		\
	    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));			\
}									\
static __inline TYPE							\
atomic_fetchadd_##NAME(volatile TYPE *p, TYPE v)			\
{									\
	return (__atomic_fetch_add(p, v, __ATOMIC_SEQ_CST));		\
}									\
static __inline TYPE							\
atomic_readandclear_##NAME(volatile TYPE *p)				\
{									\
	return (__atomic_exchange_n(p, 0, __ATOMIC_SEQ_CST));		\
}									\
static __inline TYPE							\
atomic_load_acq_##NAME(volatile TYPE *p)				\
{									\
	return (__atomic_load_n(p, __ATOMIC_SEQ_CST));			\
}									\
static __inline void							\
atomic_store_rel_##NAME(volatile TYPE *p, TYPE v)			\
{									\
	__atomic_store_n(p, v, __ATOMIC_SEQ_CST);			\
}

ATOMIC_OPS(char, u_char, b)
ATOMIC_OPS(short, u_short, h)
ATOMIC_OPS(int, u_int, w)
ATOMIC_OPS(long, u_long, x)

#undef ATOMIC_OPS

/*
 * Signed swap flavours used by MI code.
 */
static __inline int
atomic_swap_int(volatile int *addr, int value)
{
	return (__atomic_exchange_n(addr, value, __ATOMIC_SEQ_CST));
}

static __inline long
atomic_swap_long(volatile long *addr, long value)
{
	return (__atomic_exchange_n(addr, value, __ATOMIC_SEQ_CST));
}

static __inline void *
atomic_swap_ptr(volatile void **addr, void *value)
{
	return (__atomic_exchange_n((void * volatile *)addr, value,
	    __ATOMIC_SEQ_CST));
}

/*
 * Atomic compare and exchange returning the previous contents of *_p
 * rather than a success indication.
 */
static __inline int
atomic_cmpxchg_int(volatile u_int *_p, u_int _old, u_int _new)
{
	(void)__atomic_compare_exchange_n(_p, &_old, _new, 0,
	    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
	return (_old);
}

/*
 * As above for longs, but returns non-zero on success (x86_64 naming).
 */
static __inline int
atomic_cmpxchg_long_test(volatile u_long *_p, u_long _old, u_long _new)
{
	return (__atomic_compare_exchange_n(_p, &_old, _new, 0,
	    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
}

static __inline void *
atomic_cas_ptr(volatile void *_p, void *_old, void *_new)
{
	(void)__atomic_compare_exchange_n((void * volatile *)_p, &_old, _new,
	    0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
	return (_old);
}

/*
 * Attempt to obtain/release a poll-style token held in bit 0.
 */
static __inline int
atomic_poll_acquire_int(volatile u_int *p)
{
	return ((__atomic_fetch_or(p, 1, __ATOMIC_SEQ_CST) & 1) == 0);
}

static __inline void
atomic_poll_release_int(volatile u_int *p)
{
	(void)__atomic_fetch_and(p, ~1U, __ATOMIC_SEQ_CST);
}

/*
 * Atomically test and set/clear bit (v & mask) of *p.  Returns the
 * previous state of the bit.
 */
static __inline int
atomic_testandset_int(volatile u_int *p, u_int v)
{
	u_int m = 1U << (v & 0x1f);

	return ((__atomic_fetch_or(p, m, __ATOMIC_SEQ_CST) & m) != 0);
}

static __inline int
atomic_testandset_long(volatile u_long *p, u_long v)
{
	u_long m = 1UL << (v & 0x3f);

	return ((__atomic_fetch_or(p, m, __ATOMIC_SEQ_CST) & m) != 0);
}

static __inline int
atomic_testandclear_int(volatile u_int *p, u_int v)
{
	u_int m = 1U << (v & 0x1f);

	return ((__atomic_fetch_and(p, ~m, __ATOMIC_SEQ_CST) & m) != 0);
}

static __inline int
atomic_testandclear_long(volatile u_long *p, u_long v)
{
	u_long m = 1UL << (v & 0x3f);

	return ((__atomic_fetch_and(p, ~m, __ATOMIC_SEQ_CST) & m) != 0);
}

/*
 * Interrupt-safe serialization helpers (see sys/serialize.h).
 *
 * These mirror the x86_64 implementation bit for bit:
 *	bit 31		the lock itself
 *	bit 30		interrupt handler disabled
 *	bits 0-29	count of waiters
 */
/* __atomic_intr_t is a volatile int, see <machine/stdint.h> */
#define	_ATOMIC_INTR_P(p)	((volatile u_int *)(p))

#define	_ATOMIC_INTR_LOCK	0x80000000U
#define	_ATOMIC_INTR_HDIS	0x40000000U
#define	_ATOMIC_INTR_WAITMASK	0x3fffffffU

static __inline void
atomic_intr_init(__atomic_intr_t *p)
{
	*p = 0;
}

/*
 * Disable the handler.  Returns non-zero if the lock is currently held.
 */
static __inline int
atomic_intr_handler_disable(__atomic_intr_t *p)
{
	return ((int)(__atomic_or_fetch(_ATOMIC_INTR_P(p), _ATOMIC_INTR_HDIS,
	    __ATOMIC_SEQ_CST) & _ATOMIC_INTR_LOCK));
}

static __inline void
atomic_intr_handler_enable(__atomic_intr_t *p)
{
	(void)__atomic_fetch_and(_ATOMIC_INTR_P(p), ~_ATOMIC_INTR_HDIS, __ATOMIC_SEQ_CST);
}

/*
 * NOTE: as on x86_64 this returns non-zero when the handler is
 * DISABLED, despite the name.
 */
static __inline int
atomic_intr_handler_is_enabled(__atomic_intr_t *p)
{
	return ((int)(__atomic_load_n(_ATOMIC_INTR_P(p), __ATOMIC_SEQ_CST) &
	    _ATOMIC_INTR_HDIS));
}

static __inline void
atomic_intr_cond_inc(__atomic_intr_t *p)
{
	(void)__atomic_fetch_add(_ATOMIC_INTR_P(p), 1, __ATOMIC_SEQ_CST);
}

static __inline void
atomic_intr_cond_dec(__atomic_intr_t *p)
{
	(void)__atomic_fetch_sub(_ATOMIC_INTR_P(p), 1, __ATOMIC_SEQ_CST);
}

/*
 * Acquire the lock.  If it is contended, register as a waiter and call
 * func(arg) until the lock is obtained.
 */
static __inline void
atomic_intr_cond_enter(__atomic_intr_t *p, void (*func)(void *), void *arg)
{
	if ((__atomic_fetch_or(_ATOMIC_INTR_P(p), _ATOMIC_INTR_LOCK, __ATOMIC_SEQ_CST) &
	    _ATOMIC_INTR_LOCK) == 0)
		return;
	(void)__atomic_fetch_add(_ATOMIC_INTR_P(p), 1, __ATOMIC_SEQ_CST);
	while (__atomic_fetch_or(_ATOMIC_INTR_P(p), _ATOMIC_INTR_LOCK, __ATOMIC_SEQ_CST) &
	    _ATOMIC_INTR_LOCK)
		func(arg);
	(void)__atomic_fetch_sub(_ATOMIC_INTR_P(p), 1, __ATOMIC_SEQ_CST);
}

/*
 * Attempt to enter the interrupt condition variable.  Returns zero on
 * success, 1 on failure.
 */
static __inline int
atomic_intr_cond_try(__atomic_intr_t *p)
{
	return ((__atomic_fetch_or(_ATOMIC_INTR_P(p), _ATOMIC_INTR_LOCK, __ATOMIC_SEQ_CST) &
	    _ATOMIC_INTR_LOCK) != 0);
}

static __inline int
atomic_intr_cond_test(__atomic_intr_t *p)
{
	return ((int)(*p & _ATOMIC_INTR_LOCK));
}

/*
 * Release the lock and call func(arg) if anyone is waiting on it.
 */
static __inline void
atomic_intr_cond_exit(__atomic_intr_t *p, void (*func)(void *), void *arg)
{
	if (__atomic_and_fetch(_ATOMIC_INTR_P(p), ~_ATOMIC_INTR_LOCK, __ATOMIC_SEQ_CST) &
	    _ATOMIC_INTR_WAITMASK)
		func(arg);
}

/* No lock elision on aarch64 */
#define	atomic_cmpset_int_xacquire	atomic_cmpset_int
#define	atomic_cmpset_int_xrelease	atomic_cmpset_int
#define	atomic_cmpset_long_xacquire	atomic_cmpset_long
#define	atomic_cmpset_long_xrelease	atomic_cmpset_long
#define	atomic_fetchadd_int_xacquire	atomic_fetchadd_int
#define	atomic_fetchadd_int_xrelease	atomic_fetchadd_int
#define	atomic_fetchadd_long_xacquire	atomic_fetchadd_long
#define	atomic_fetchadd_long_xrelease	atomic_fetchadd_long
#define	atomic_add_int_xacquire		atomic_add_int
#define	atomic_add_int_xrelease		atomic_add_int
#define	atomic_add_long_xacquire	atomic_add_long
#define	atomic_add_long_xrelease	atomic_add_long
#define	atomic_subtract_int_xacquire	atomic_subtract_int
#define	atomic_subtract_int_xrelease	atomic_subtract_int
#define	atomic_subtract_long_xacquire	atomic_subtract_long
#define	atomic_subtract_long_xrelease	atomic_subtract_long
#define	atomic_set_int_xacquire		atomic_set_int
#define	atomic_set_int_xrelease		atomic_set_int
#define	atomic_clear_int_xacquire	atomic_clear_int
#define	atomic_clear_int_xrelease	atomic_clear_int

/* Acquire and release variants are identical to the normal ones. */
#define	_ATOMIC_ACQREL(T)						\
	static __inline void atomic_set_acq_##T(volatile u_##T *p, u_##T v)	\
	{ atomic_set_##T(p, v); }					\
	static __inline void atomic_set_rel_##T(volatile u_##T *p, u_##T v)	\
	{ atomic_set_##T(p, v); }					\
	static __inline void atomic_clear_acq_##T(volatile u_##T *p, u_##T v)	\
	{ atomic_clear_##T(p, v); }					\
	static __inline void atomic_clear_rel_##T(volatile u_##T *p, u_##T v)	\
	{ atomic_clear_##T(p, v); }					\
	static __inline void atomic_add_acq_##T(volatile u_##T *p, u_##T v)	\
	{ atomic_add_##T(p, v); }					\
	static __inline void atomic_add_rel_##T(volatile u_##T *p, u_##T v)	\
	{ atomic_add_##T(p, v); }					\
	static __inline void atomic_subtract_acq_##T(volatile u_##T *p, u_##T v) \
	{ atomic_subtract_##T(p, v); }					\
	static __inline void atomic_subtract_rel_##T(volatile u_##T *p, u_##T v) \
	{ atomic_subtract_##T(p, v); }
_ATOMIC_ACQREL(char)
_ATOMIC_ACQREL(short)
_ATOMIC_ACQREL(int)
_ATOMIC_ACQREL(long)
#undef _ATOMIC_ACQREL

#define	atomic_cmpset_acq_int		atomic_cmpset_int
#define	atomic_cmpset_rel_int		atomic_cmpset_int
#define	atomic_cmpset_acq_long		atomic_cmpset_long
#define	atomic_cmpset_rel_long		atomic_cmpset_long

/* cpumask_t is 64 bits */
#define atomic_set_cpumask		atomic_set_long
#define atomic_clear_cpumask		atomic_clear_long
#define atomic_cmpset_cpumask		atomic_cmpset_long
#define atomic_store_rel_cpumask	atomic_store_rel_long
#define atomic_load_acq_cpumask		atomic_load_acq_long

/* Operations on 8-bit bytes. */
#define atomic_set_8		atomic_set_char
#define atomic_set_acq_8	atomic_set_acq_char
#define atomic_set_rel_8	atomic_set_rel_char
#define atomic_clear_8		atomic_clear_char
#define atomic_clear_acq_8	atomic_clear_acq_char
#define atomic_clear_rel_8	atomic_clear_rel_char
#define atomic_add_8		atomic_add_char
#define atomic_add_acq_8	atomic_add_acq_char
#define atomic_add_rel_8	atomic_add_rel_char
#define atomic_subtract_8	atomic_subtract_char
#define atomic_subtract_acq_8	atomic_subtract_acq_char
#define atomic_subtract_rel_8	atomic_subtract_rel_char
#define atomic_load_acq_8	atomic_load_acq_char
#define atomic_store_rel_8	atomic_store_rel_char
#define atomic_fcmpset_8	atomic_fcmpset_char

/* Operations on 16-bit words. */
#define atomic_set_16		atomic_set_short
#define atomic_set_acq_16	atomic_set_acq_short
#define atomic_set_rel_16	atomic_set_rel_short
#define atomic_clear_16		atomic_clear_short
#define atomic_clear_acq_16	atomic_clear_acq_short
#define atomic_clear_rel_16	atomic_clear_rel_short
#define atomic_add_16		atomic_add_short
#define atomic_add_acq_16	atomic_add_acq_short
#define atomic_add_rel_16	atomic_add_rel_short
#define atomic_subtract_16	atomic_subtract_short
#define atomic_subtract_acq_16	atomic_subtract_acq_short
#define atomic_subtract_rel_16	atomic_subtract_rel_short
#define atomic_load_acq_16	atomic_load_acq_short
#define atomic_store_rel_16	atomic_store_rel_short
#define atomic_fcmpset_16	atomic_fcmpset_short

/* Operations on 32-bit double words. */
#define atomic_set_32		atomic_set_int
#define atomic_set_acq_32	atomic_set_acq_int
#define atomic_set_rel_32	atomic_set_rel_int
#define atomic_clear_32		atomic_clear_int
#define atomic_clear_acq_32	atomic_clear_acq_int
#define atomic_clear_rel_32	atomic_clear_rel_int
#define atomic_add_32		atomic_add_int
#define atomic_add_acq_32	atomic_add_acq_int
#define atomic_add_rel_32	atomic_add_rel_int
#define atomic_subtract_32	atomic_subtract_int
#define atomic_subtract_acq_32	atomic_subtract_acq_int
#define atomic_subtract_rel_32	atomic_subtract_rel_int
#define atomic_load_acq_32	atomic_load_acq_int
#define atomic_store_rel_32	atomic_store_rel_int
#define atomic_cmpset_32	atomic_cmpset_int
#define atomic_fcmpset_32	atomic_fcmpset_int
#define atomic_cmpset_acq_32	atomic_cmpset_acq_int
#define atomic_cmpset_rel_32	atomic_cmpset_rel_int
#define atomic_readandclear_32	atomic_readandclear_int
#define atomic_fetchadd_32	atomic_fetchadd_int

/* Operations on 64-bit quad words. */
#define atomic_load_acq_64	atomic_load_acq_long
#define atomic_store_rel_64	atomic_store_rel_long
#define atomic_swap_64		atomic_swap_long
#define atomic_fetchadd_64	atomic_fetchadd_long
#define atomic_add_64		atomic_add_long
#define atomic_subtract_64	atomic_subtract_long
#define atomic_cmpset_64	atomic_cmpset_long
#define atomic_fcmpset_64	atomic_fcmpset_long
#define atomic_set_64		atomic_set_long
#define atomic_clear_64		atomic_clear_long

/* Operations on pointers. */
#define atomic_set_ptr(p, v) \
	atomic_set_long((volatile u_long *)(p), (u_long)(v))
#define atomic_set_acq_ptr(p, v) \
	atomic_set_acq_long((volatile u_long *)(p), (u_long)(v))
#define atomic_set_rel_ptr(p, v) \
	atomic_set_rel_long((volatile u_long *)(p), (u_long)(v))
#define atomic_clear_ptr(p, v) \
	atomic_clear_long((volatile u_long *)(p), (u_long)(v))
#define atomic_clear_acq_ptr(p, v) \
	atomic_clear_acq_long((volatile u_long *)(p), (u_long)(v))
#define atomic_clear_rel_ptr(p, v) \
	atomic_clear_rel_long((volatile u_long *)(p), (u_long)(v))
#define atomic_add_ptr(p, v) \
	atomic_add_long((volatile u_long *)(p), (u_long)(v))
#define atomic_add_acq_ptr(p, v) \
	atomic_add_acq_long((volatile u_long *)(p), (u_long)(v))
#define atomic_add_rel_ptr(p, v) \
	atomic_add_rel_long((volatile u_long *)(p), (u_long)(v))
#define atomic_subtract_ptr(p, v) \
	atomic_subtract_long((volatile u_long *)(p), (u_long)(v))
#define atomic_subtract_acq_ptr(p, v) \
	atomic_subtract_acq_long((volatile u_long *)(p), (u_long)(v))
#define atomic_subtract_rel_ptr(p, v) \
	atomic_subtract_rel_long((volatile u_long *)(p), (u_long)(v))
#define atomic_load_acq_ptr(p) \
	atomic_load_acq_long((volatile u_long *)(p))
#define atomic_store_rel_ptr(p, v) \
	atomic_store_rel_long((volatile u_long *)(p), (v))
#define atomic_cmpset_ptr(dst, old, new) \
	atomic_cmpset_long((volatile u_long *)(dst), (u_long)(old), \
				(u_long)(new))
#define atomic_fcmpset_ptr(dst, old, new) \
	atomic_fcmpset_long((volatile u_long *)(dst), (u_long *)(old), \
				(u_long)(new))
#define atomic_cmpset_acq_ptr(dst, old, new) \
	atomic_cmpset_acq_long((volatile u_long *)(dst), (u_long)(old), \
				(u_long)(new))
#define atomic_cmpset_rel_ptr(dst, old, new) \
	atomic_cmpset_rel_long((volatile u_long *)(dst), (u_long)(old), \
				(u_long)(new))
#define atomic_readandclear_ptr(p) \
	atomic_readandclear_long((volatile u_long *)(p))

#endif /* ! _CPU_ATOMIC_H_ */
