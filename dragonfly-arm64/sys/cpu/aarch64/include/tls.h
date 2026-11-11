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
 * Thread control block.
 *
 * The aarch64 ELF TLS ABI is variant I: the thread pointer (TPIDR_EL0)
 * addresses a 16-byte header whose first word is the DTV pointer, and
 * the static TLS blocks follow it at positive offsets.  Code compiled
 * for the local-exec and initial-exec models hardwires that layout, so
 * DragonFly follows it here, unlike on x86_64 (variant II).
 *
 * DragonFly's other TCB fields go in front of the ABI header: the
 * thread pointer addresses tcb_dtv, not the start of the structure.
 * tls_get_tcb() and tls_set_tcb() hide the difference.
 */
struct tls_tcb {
	struct tls_tcb *tcb_self;	/* pointer to self */
	int *tcb_errno_p;		/* pointer to per-thread errno */
	void *tcb_segstack;		/* used for segmented stacks */
	void *tcb_unused[3];
	/* The thread pointer points here; the ABI fixes these two. */
	void *tcb_dtv;			/* Dynamic Thread Vector */
	void *tcb_pthread;		/* thread library's data */
};

/* Offset of the thread pointer within struct tls_tcb. */
#define	TLS_TCB_TP_OFFSET	__offsetof(struct tls_tcb, tcb_dtv)
/* Size of the ABI header at the thread pointer. */
#define	TLS_TP_HDR_SIZE		16

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

#define RTLD_STATIC_TLS_VARIANT_I

/* Get the current TCB. */
static __inline struct tls_tcb *
tls_get_tcb(void)
{
	char *tp;

	__asm __volatile("mrs %0, tpidr_el0" : "=r" (tp));
	if (tp == 0)
		return (0);
	return ((struct tls_tcb *)(tp - TLS_TCB_TP_OFFSET));
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
	char *tp = tcb ? (char *)tcb + TLS_TCB_TP_OFFSET : (char *)0;

	__asm __volatile("msr tpidr_el0, %0" : : "r" (tp));
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
