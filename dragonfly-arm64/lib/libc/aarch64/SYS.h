/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * System call stubs for aarch64.  The kernel takes the call number in
 * x8 and arguments in x0-x7, and returns the result in x0 (and x1);
 * on failure it sets the carry flag and puts the errno value in x0.
 */
#include <sys/syscall.h>
#include <machine/asm.h>

#define	KERNCALL(n)	mov x8, #SYS_##n; svc #0

#define	RSYSCALL(x)	ENTRY(__sys_##x);				\
			.weak CNAME(x);					\
			.set CNAME(x),CNAME(__sys_##x);			\
			.weak CNAME(_##x);				\
			.set CNAME(_##x),CNAME(__sys_##x);		\
			KERNCALL(x); b.cs HIDENAME(cerror); ret;	\
			END(__sys_##x)

#define	PSEUDO(x)	ENTRY(__sys_##x);				\
			.weak CNAME(_##x);				\
			.set CNAME(_##x),CNAME(__sys_##x);		\
			KERNCALL(x); b.cs HIDENAME(cerror); ret;	\
			END(__sys_##x)
