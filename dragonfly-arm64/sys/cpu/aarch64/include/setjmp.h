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

#ifndef _CPU_SETJMP_H_
#define	_CPU_SETJMP_H_

#include <sys/cdefs.h>

/*
 * x19-x30, sp, d8-d15 and the signal mask bookkeeping, with headroom.
 */
#define	_JBLEN	32

/*
 * Layout used by libc (as FreeBSD/arm64): magic, sp, x19-x29, lr,
 * d8-d15, then the signal mask (a sigset_t, two longs) at _JB_SIGMASK.
 */
#define	_JB_MAGIC__SETJMP	0xfb5d25837d7ff700
#define	_JB_MAGIC_SETJMP	0xfb5d25837d7ff701
#define	_JB_SIGMASK		22

#ifndef __ASSEMBLER__
#if __POSIX_VISIBLE
typedef	struct _sigjmp_buf { long _sjb[_JBLEN]; } sigjmp_buf[1];
#endif

typedef	struct _jmp_buf { long _jb[_JBLEN]; } jmp_buf[1];
#endif

#endif /* !_CPU_SETJMP_H_ */
