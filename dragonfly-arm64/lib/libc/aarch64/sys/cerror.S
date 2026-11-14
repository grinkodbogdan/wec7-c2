/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include "SYS.h"

	.globl	HIDENAME(cerror)
	.type	HIDENAME(cerror), %function

/* x0 holds the errno value; return -1 in x0 and x1. */
HIDENAME(cerror):
	stp	x29, x30, [sp, #-16]!
	mov	x29, sp
	bl	__cerror_set
	mov	x1, x0
	ldp	x29, x30, [sp], #16
	ret

	.section .note.GNU-stack,"",%progbits
