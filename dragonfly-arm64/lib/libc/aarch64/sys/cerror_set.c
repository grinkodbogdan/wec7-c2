/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <errno.h>

long __cerror_set(int);

/*
 * Tail of every failed system call stub (see cerror.S).  In C so that
 * the compiler picks the right TLS access for errno, static or PIC.
 */
long
__cerror_set(int error)
{
	errno = error;
	return (-1);
}
