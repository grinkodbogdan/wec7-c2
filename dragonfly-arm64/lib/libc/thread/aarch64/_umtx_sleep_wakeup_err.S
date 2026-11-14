/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * _umtx_sleep_err() and _umtx_wakeup_err(): the system calls, returning
 * the errno value itself on failure instead of setting errno.
 */
#include <machine/asm.h>
#include <sys/syscall.h>

ENTRY(_umtx_sleep_err)
	mov	x8, #SYS_umtx_sleep
	svc	#0
	ret
END(_umtx_sleep_err)

ENTRY(_umtx_wakeup_err)
	mov	x8, #SYS_umtx_wakeup
	svc	#0
	ret
END(_umtx_wakeup_err)

	.section .note.GNU-stack,"",%progbits
