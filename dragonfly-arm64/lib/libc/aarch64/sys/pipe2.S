/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include "SYS.h"

/* The kernel returns both descriptors in x0 and x1. */
	.weak	_pipe2
	.set	_pipe2,__sys_pipe2
	.weak	pipe2
	.set	pipe2,__sys_pipe2
ENTRY(__sys_pipe2)
	mov	x9, x0			/* the kernel preserves x9 */
	KERNCALL(pipe2)
	b.cs	HIDENAME(cerror)
	stp	w0, w1, [x9]
	mov	x0, #0
	ret
END(__sys_pipe2)

	.section .note.GNU-stack,"",%progbits
