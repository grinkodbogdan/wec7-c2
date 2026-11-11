/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include "SYS.h"

/* The kernel returns both descriptors in x0 and x1. */
	.weak	_pipe
	.set	_pipe,__sys_pipe
	.weak	pipe
	.set	pipe,__sys_pipe
ENTRY(__sys_pipe)
	mov	x9, x0			/* the kernel preserves x9 */
	KERNCALL(pipe)
	b.cs	HIDENAME(cerror)
	stp	w0, w1, [x9]
	mov	x0, #0
	ret
END(__sys_pipe)

	.section .note.GNU-stack,"",%progbits
