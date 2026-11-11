/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include "SYS.h"

	.weak	_setlogin
	.set	_setlogin,__sys_setlogin
	.weak	setlogin
	.set	setlogin,__sys_setlogin
ENTRY(__sys_setlogin)
	KERNCALL(setlogin)
	b.cs	HIDENAME(cerror)
	adrp	x1, :got:_logname_valid
	ldr	x1, [x1, :got_lo12:_logname_valid]
	str	wzr, [x1]
	ret
END(__sys_setlogin)

	.section .note.GNU-stack,"",%progbits
