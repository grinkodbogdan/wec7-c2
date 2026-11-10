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

#ifndef _CPU_PROFILE_H_
#define	_CPU_PROFILE_H_

#include <sys/cdefs.h>

#define	FUNCTION_ALIGNMENT	4

#define	_MCOUNT_DECL \
static void _mcount(u_long frompc, u_long selfpc) __used; \
static void _mcount

/*
 * Profiling entry stub.  Not implemented for aarch64 yet; -pg builds
 * are unsupported.
 */
#define	MCOUNT

typedef	u_long	fptrdiff_t;
typedef	u_long	uintfptr_t;

/*
 * An unsigned integral type that can hold non-negative difference between
 * function pointers.
 */
typedef	u_long	fptrint_t;

#endif /* !_CPU_PROFILE_H_ */
