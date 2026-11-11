/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include "SYS.h"

	.globl	CNAME(_end)
	.globl	HIDENAME(minbrk)
	.globl	HIDENAME(curbrk)

	.data
	.balign	8
HIDENAME(minbrk):	.quad	CNAME(_end)
HIDENAME(curbrk):	.quad	CNAME(_end)
	.text

/*
 * void *fake_sbrk(intptr_t incr): the old break-based sbrk(), for
 * gen/ovsbrk.c.  x9-x13 survive the system call.
 */
ENTRY(fake_sbrk)
	mov	x10, x0
	adrp	x11, :got:HIDENAME(curbrk)
	ldr	x11, [x11, :got_lo12:HIDENAME(curbrk)]
	ldr	x12, [x11]
	cbz	x10, 1f
	add	x0, x12, x10
	KERNCALL(break)
	b.cs	HIDENAME(cerror)
	ldr	x12, [x11]
	add	x13, x12, x10
	str	x13, [x11]
1:	mov	x0, x12
	ret
END(fake_sbrk)

	.section .note.GNU-stack,"",%progbits
