/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include "SYS.h"

/*
 * The child borrows the parent's stack until it execs or exits, so
 * the stub must not touch the stack.  lr survives the system call.
 */
	.weak	_vfork
	.set	_vfork,__sys_vfork
	.weak	vfork
	.set	vfork,__sys_vfork
ENTRY(__sys_vfork)
	KERNCALL(vfork)
	b.cs	HIDENAME(cerror)
	ret
END(__sys_vfork)

	.section .note.GNU-stack,"",%progbits
