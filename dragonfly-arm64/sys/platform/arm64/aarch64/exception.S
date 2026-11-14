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

/*
 * EL1 exception vectors.  Every entry builds a struct trapframe
 * (see <cpu/frame.h>) on the kernel stack and calls into C.
 *
 * A thread's kernel stack is empty while it runs in user mode: SP_EL1
 * was left pointing at the thread's pcb by the last return to user
 * mode, so the frame built on entry from EL0 always lands directly
 * below the pcb, which is where lwp_md.md_regs expects it.
 */

#include <cpu/asm.h>
#include "assym.s"

.macro	save_frame
	sub	sp, sp, #TF_SIZE
	stp	x0, x1, [sp, #(TF_X + 0 * 8)]
	stp	x2, x3, [sp, #(TF_X + 2 * 8)]
	stp	x4, x5, [sp, #(TF_X + 4 * 8)]
	stp	x6, x7, [sp, #(TF_X + 6 * 8)]
	stp	x8, x9, [sp, #(TF_X + 8 * 8)]
	stp	x10, x11, [sp, #(TF_X + 10 * 8)]
	stp	x12, x13, [sp, #(TF_X + 12 * 8)]
	stp	x14, x15, [sp, #(TF_X + 14 * 8)]
	stp	x16, x17, [sp, #(TF_X + 16 * 8)]
	stp	x18, x19, [sp, #(TF_X + 18 * 8)]
	stp	x20, x21, [sp, #(TF_X + 20 * 8)]
	stp	x22, x23, [sp, #(TF_X + 22 * 8)]
	stp	x24, x25, [sp, #(TF_X + 24 * 8)]
	stp	x26, x27, [sp, #(TF_X + 26 * 8)]
	stp	x28, x29, [sp, #(TF_X + 28 * 8)]
	add	x10, sp, #TF_SIZE
	stp	x10, x30, [sp, #TF_SP]
	mrs	x10, elr_el1
	mrs	x11, spsr_el1
	stp	x10, x11, [sp, #TF_ELR]
	mrs	x10, esr_el1
	mrs	x11, far_el1
	stp	x10, x11, [sp, #TF_ESR]
.endm

/*
 * As save_frame, for an exception taken from EL0: the interrupted
 * stack pointer is SP_EL0 rather than our own.
 */
.macro	save_frame_el0
	sub	sp, sp, #TF_SIZE
	stp	x0, x1, [sp, #(TF_X + 0 * 8)]
	stp	x2, x3, [sp, #(TF_X + 2 * 8)]
	stp	x4, x5, [sp, #(TF_X + 4 * 8)]
	stp	x6, x7, [sp, #(TF_X + 6 * 8)]
	stp	x8, x9, [sp, #(TF_X + 8 * 8)]
	stp	x10, x11, [sp, #(TF_X + 10 * 8)]
	stp	x12, x13, [sp, #(TF_X + 12 * 8)]
	stp	x14, x15, [sp, #(TF_X + 14 * 8)]
	stp	x16, x17, [sp, #(TF_X + 16 * 8)]
	stp	x18, x19, [sp, #(TF_X + 18 * 8)]
	stp	x20, x21, [sp, #(TF_X + 20 * 8)]
	stp	x22, x23, [sp, #(TF_X + 22 * 8)]
	stp	x24, x25, [sp, #(TF_X + 24 * 8)]
	stp	x26, x27, [sp, #(TF_X + 26 * 8)]
	stp	x28, x29, [sp, #(TF_X + 28 * 8)]
	mrs	x10, sp_el0
	stp	x10, x30, [sp, #TF_SP]
	mrs	x10, elr_el1
	mrs	x11, spsr_el1
	stp	x10, x11, [sp, #TF_ELR]
	mrs	x10, esr_el1
	mrs	x11, far_el1
	stp	x10, x11, [sp, #TF_ESR]
	mov	x29, #0
.endm

.macro	restore_frame_el0
	ldp	x10, x11, [sp, #TF_ELR]
	msr	elr_el1, x10
	msr	spsr_el1, x11
	ldp	x10, x30, [sp, #TF_SP]
	msr	sp_el0, x10
	ldp	x0, x1, [sp, #(TF_X + 0 * 8)]
	ldp	x2, x3, [sp, #(TF_X + 2 * 8)]
	ldp	x4, x5, [sp, #(TF_X + 4 * 8)]
	ldp	x6, x7, [sp, #(TF_X + 6 * 8)]
	ldp	x8, x9, [sp, #(TF_X + 8 * 8)]
	ldp	x10, x11, [sp, #(TF_X + 10 * 8)]
	ldp	x12, x13, [sp, #(TF_X + 12 * 8)]
	ldp	x14, x15, [sp, #(TF_X + 14 * 8)]
	ldp	x16, x17, [sp, #(TF_X + 16 * 8)]
	ldp	x18, x19, [sp, #(TF_X + 18 * 8)]
	ldp	x20, x21, [sp, #(TF_X + 20 * 8)]
	ldp	x22, x23, [sp, #(TF_X + 22 * 8)]
	ldp	x24, x25, [sp, #(TF_X + 24 * 8)]
	ldp	x26, x27, [sp, #(TF_X + 26 * 8)]
	ldp	x28, x29, [sp, #(TF_X + 28 * 8)]
	add	sp, sp, #TF_SIZE
.endm

.macro	restore_frame
	ldp	x10, x11, [sp, #TF_ELR]
	msr	elr_el1, x10
	msr	spsr_el1, x11
	ldr	x30, [sp, #TF_LR]
	ldp	x0, x1, [sp, #(TF_X + 0 * 8)]
	ldp	x2, x3, [sp, #(TF_X + 2 * 8)]
	ldp	x4, x5, [sp, #(TF_X + 4 * 8)]
	ldp	x6, x7, [sp, #(TF_X + 6 * 8)]
	ldp	x8, x9, [sp, #(TF_X + 8 * 8)]
	ldp	x10, x11, [sp, #(TF_X + 10 * 8)]
	ldp	x12, x13, [sp, #(TF_X + 12 * 8)]
	ldp	x14, x15, [sp, #(TF_X + 14 * 8)]
	ldp	x16, x17, [sp, #(TF_X + 16 * 8)]
	ldp	x18, x19, [sp, #(TF_X + 18 * 8)]
	ldp	x20, x21, [sp, #(TF_X + 20 * 8)]
	ldp	x22, x23, [sp, #(TF_X + 22 * 8)]
	ldp	x24, x25, [sp, #(TF_X + 24 * 8)]
	ldp	x26, x27, [sp, #(TF_X + 26 * 8)]
	ldp	x28, x29, [sp, #(TF_X + 28 * 8)]
	add	sp, sp, #TF_SIZE
.endm

.macro	vector name
	.balign	0x80
	b	\name
.endm

	.text
	.balign	0x800
	.globl	exception_vectors
exception_vectors:
	vector	el1_bad			/* EL1t sync */
	vector	el1_bad			/* EL1t irq */
	vector	el1_bad			/* EL1t fiq */
	vector	el1_bad			/* EL1t serror */

	vector	el1h_sync		/* EL1h sync */
	vector	el1h_irq		/* EL1h irq */
	vector	el1_bad			/* EL1h fiq */
	vector	el1_bad			/* EL1h serror */

	vector	el0_sync		/* EL0 64-bit sync */
	vector	el0_irq			/* EL0 64-bit irq */
	vector	el0_bad			/* EL0 64-bit fiq */
	vector	el0_bad			/* EL0 64-bit serror */

	vector	el0_bad			/* EL0 32-bit sync */
	vector	el0_bad			/* EL0 32-bit irq */
	vector	el0_bad			/* EL0 32-bit fiq */
	vector	el0_bad			/* EL0 32-bit serror */
	.balign	0x80

el1h_sync:
	save_frame
	mov	x0, sp
	bl	trap
	restore_frame
	eret

el1h_irq:
	save_frame
	mov	x0, sp
	bl	arm64_intr
	restore_frame
	eret

/*
 * System calls, faults and other synchronous exceptions from user mode.
 */
el0_sync:
	save_frame_el0
	mov	x0, sp
	bl	trap_el0
	b	user_return

el0_irq:
	save_frame_el0
	mov	x0, sp
	bl	arm64_intr
	/* FALLTHROUGH */

/*
 * Return to user mode with the trapframe at sp.  arm64_user_return()
 * runs pending ASTs (signals, rescheduling) and comes back with
 * interrupts masked and nothing left to do.  Also the tail of
 * fork_trampoline.
 */
	.globl	user_return
user_return:
	mov	x0, sp
	bl	arm64_user_return
	restore_frame_el0
	eret

el1_bad:
	save_frame
	mov	x0, sp
	mov	x1, #1
	bl	trap_fatal
1:	wfi
	b	1b

el0_bad:
	save_frame
	mov	x0, sp
	mov	x1, #0
	bl	trap_fatal
2:	wfi
	b	2b

/*
 * int64_t psci_call4(uint64_t fn, uint64_t a1, uint64_t a2, uint64_t a3,
 *     int use_smc)
 */
	.globl	psci_call4
psci_call4:
	cbnz	x4, 1f
	hvc	#0
	ret
1:	smc	#0
	ret

/*
 * int64_t psci_call(uint64_t fn, uint64_t a1, uint64_t a2, int use_smc)
 */
	.globl	psci_call
psci_call:
	cbnz	x3, 3f
	hvc	#0
	ret
3:	smc	#0
	ret

/*
 * Signal trampoline, copied out to user space by exec.  sendsig()
 * points the pc here with sp at the struct sigframe and the handler's
 * arguments already in x0-x3.  Call the handler, then hand the saved
 * context back to the kernel with sigreturn(2).
 */
	.globl	sigcode, esigcode
	.balign	4
sigcode:
	ldr	x9, [sp, #SIGF_HANDLER]
	blr	x9
	add	x0, sp, #SIGF_UC
	mov	x8, #344		/* SYS_sigreturn */
	svc	#0
	brk	#0xdead			/* sigreturn does not return */
esigcode:

	.data
	.globl	szsigcode
szsigcode:
	.long	esigcode - sigcode

/*
 * kern/link_elf.c walks the kernel's own dynamic section to find its
 * symbol table.  The kernel is linked statically for now, so give it
 * an empty one (a lone DT_NULL): kernel modules cannot resolve symbols
 * against the kernel until this is real.
 */
	.balign	8
	.globl	_DYNAMIC
_DYNAMIC:
	.quad	0, 0
