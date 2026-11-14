/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#ifndef _PTHREAD_MD_H_
#define	_PTHREAD_MD_H_

#include <stddef.h>
#include <sys/types.h>
#include <machine/tls.h>

struct __pthread_s;

#define	CPU_SPINWAIT		__asm __volatile("yield");

#define	atomic_cmpset_acq_int	atomic_cmpset_int

/*
 * The constructors.
 */
struct tls_tcb	*_tcb_ctor(struct __pthread_s *, int);
void		_tcb_dtor(struct tls_tcb *tcb);

#endif
