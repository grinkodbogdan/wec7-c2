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

#ifndef	_CPU_TLS_H_
#define	_CPU_TLS_H_

#ifndef _SYS_TYPES_H_
#include <sys/types.h>
#endif
#ifndef _SYS_TLS_H_
#include <sys/tls.h>
#endif

/*
 * NOTE: the tcb_{self,dtv,pthread,errno) fields must be declared
 * in the structure in the specified order as assembly will access the
 * fields with a hardwired offset.
 *
 * Outside of this file, the system call layer generation will hardwire
 * the offset for tcb_errno.
 */
struct tls_tcb {
	struct tls_tcb *tcb_self;	/* pointer to self*/
	void *tcb_dtv;			/* Dynamic Thread Vector */
	void *tcb_pthread;		/* thread library's data*/
	int *tcb_errno_p;		/* pointer to per-thread errno */
	void *tcb_segstack;		/* used for segmented stacks */
	void *tcb_unused[3];
};

struct tls_dtv {
	uintptr_t dtv_generation;
	uintptr_t dtv_max_index;
	void *dtv_offset[__ARRAY_ZERO];
};

#define	RTLD_TCB_HAS_SELF_POINTER
#define	RTLD_STATIC_TLS_ALIGN           16
#define	RTLD_STATIC_TLS_ALIGN_MASK      (RTLD_STATIC_TLS_ALIGN - 1)
#define RTLD_STATIC_TLS_EXTRA_MIN	2048
#define RTLD_STATIC_TLS_EXTRA_MAX	(1*1024*1024*1024)
#define RTLD_STATIC_TLS_EXTRA_DEFAULT	6144	/* sysctl kern.tls_extra */

/*
 * DragonFly keeps the x86 arrangement on every architecture: the static
 * TLS block sits below the TCB (variant II) and the thread pointer
 * addresses the TCB.  On aarch64 the thread pointer is TPIDR_EL0, which
 * userland may read and write directly.
 *
 * NOTE: the aarch64 ELF TLS ABI specifies variant I.  Compiler-generated
 * local-exec/initial-exec TLS accesses assume it, so the userland port
 * must either switch DragonFly's rtld/libc to variant I for this
 * architecture or stick to the general-dynamic model.  Unresolved.
 */
#define RTLD_STATIC_TLS_VARIANT_II

/* Get the current TCB. */
static __inline struct tls_tcb *
tls_get_tcb(void)
{
	void *self;

	__asm __volatile("mrs %0, tpidr_el0" : "=r" (self));
	return (self);
}

/* Get the current thread. */
static __inline void *
tls_get_curthread(void)
{
	return (tls_get_tcb()->tcb_pthread);
}

#ifndef _KERNEL

/* Set the current TCB. */
static __inline void
tls_set_tcb(struct tls_tcb *tcb)
{
	__asm __volatile("msr tpidr_el0, %0" : : "r" (tcb));
}

struct tls_tcb	*_rtld_allocate_tls(void);
struct tls_tcb	*_libc_allocate_tls(void);
void		 _rtld_free_tls(struct tls_tcb *);
void		 _libc_free_tls(struct tls_tcb *);
void		 _rtld_call_init(void);
struct tls_tcb	*_libc_init_tls(void);
struct tls_tcb	*_init_tls(void);

#endif

#endif	/* !_CPU_TLS_H_ */
