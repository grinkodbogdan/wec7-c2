/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#ifndef _LIBC_AARCH64_STATIC_TLS_H
#define _LIBC_AARCH64_STATIC_TLS_H

#include <sys/types.h>
#include <sys/cdefs.h>

/*
 * TLS variant I: static TLS lies above the thread pointer, and
 * (offset) is the distance from the thread pointer.
 */
static __inline uintptr_t
_libc_get_static_tls_base(size_t offset)
{
	uintptr_t tp;

	__asm __volatile("mrs %0, tpidr_el0" : "=r" (tp));
	return (tp + offset);
}

#endif
