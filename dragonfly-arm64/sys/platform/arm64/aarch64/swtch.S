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
 * Thread switching.
 *
 * The protocol is the one kern/lwkt_thread.c expects on every platform
 * and that pc64's swtch.s implements:
 *
 *   - td->td_switch(ntd) saves the caller on its own stack, with the
 *     address of a "restore" function on top, records the stack pointer
 *     in td_sp and jumps to whatever restore function is on top of the
 *     new thread's stack.
 *   - A restore function is entered with x0 = the thread being switched
 *     to and x19 = the thread switched away from (the roles %rax and
 *     %rbx have on x86_64).
 *   - td_switch() returns the old thread to its caller, which hands it
 *     to lwkt_switch_return().  Restore functions that do not return
 *     to such a caller (new threads) make that call themselves.
 *
 * Switch frame, 112 bytes, lowest address first:
 *
 *	restore function, saved DAIF, x19-x28, x29, x30
 */

#include <cpu/asm.h>
#include "assym.s"

#define	SWF_SIZE	112

.macro	save_switch_frame restore
	sub	sp, sp, #SWF_SIZE
	stp	x19, x20, [sp, #16]
	stp	x21, x22, [sp, #32]
	stp	x23, x24, [sp, #48]
	stp	x25, x26, [sp, #64]
	stp	x27, x28, [sp, #80]
	stp	x29, x30, [sp, #96]
	mrs	x9, daif
	adr	x10, \restore
	stp	x10, x9, [sp]
.endm

/* Returns to the saved x30 with x0 = old thread (in x19 on entry). */
.macro	restore_switch_frame
	mov	x0, x19
	ldr	x9, [sp, #8]
	ldp	x19, x20, [sp, #16]
	ldp	x21, x22, [sp, #32]
	ldp	x23, x24, [sp, #48]
	ldp	x25, x26, [sp, #64]
	ldp	x27, x28, [sp, #80]
	ldp	x29, x30, [sp, #96]
	add	sp, sp, #SWF_SIZE
	msr	daif, x9
	ret
.endm

/* Record the outgoing thread and jump to the incoming one's restore. */
.macro	switch_stacks
	msr	daifset, #3
	mrs	x11, tpidr_el1
	ldr	x19, [x11, #GD_CURTHREAD]
	mov	x12, sp
	str	x12, [x19, #TD_SP]
	str	x0, [x11, #GD_CURTHREAD]
	ldr	x12, [x0, #TD_SP]
	mov	sp, x12
	ldr	x10, [sp]
	br	x10
.endm

/*
 * Load a user translation table base.  Address space ids are not used
 * yet, so every change of TTBR0 flushes this cpu's TLB.
 */
.macro	load_ttbr0 reg, tmp
	mrs	\tmp, ttbr0_el1
	cmp	\tmp, \reg
	b.eq	9f
	msr	ttbr0_el1, \reg
	isb
	tlbi	vmalle1
	dsb	nsh
	isb
9:
.endm

/*
 * A pure kernel thread has no user address space.  Unless it is
 * preempting (and so borrowing the interrupted thread's context), park
 * TTBR0 on the empty table so a stale user pmap is never left live.
 * Expects x0 = new thread.
 */
.macro	kernel_ttbr0
	ldr	x9, [x0, #TD_PREEMPTED]
	cbnz	x9, 8f
	adrp	x9, arm64_ttbr0_kernel
	ldr	x9, [x9, :lo12:arm64_ttbr0_kernel]
	load_ttbr0 x9, x10
8:
.endm

/*
 * thread_t cpu_lwkt_switch(thread_t ntd)
 *
 * Switch between kernel threads.
 */
ENTRY(cpu_lwkt_switch)
	save_switch_frame cpu_lwkt_restore
	switch_stacks
END(cpu_lwkt_switch)

ENTRY(cpu_lwkt_restore)
	kernel_ttbr0
	restore_switch_frame
END(cpu_lwkt_restore)

/*
 * thread_t cpu_heavy_switch(thread_t ntd)
 *
 * Switch away from a thread that has a user process context.
 *
 * TODO: save the FP/SIMD state here once the kernel lets userland use
 * it; the kernel itself is built without FP.
 */
ENTRY(cpu_heavy_switch)
	save_switch_frame cpu_heavy_restore
	switch_stacks
END(cpu_heavy_switch)

/*
 * Resume a thread with a user process context: install its address
 * space and thread pointer.  Also the first code a forked thread runs,
 * in which case the "saved" x30 is fork_trampoline (see cpu_fork()).
 */
ENTRY(cpu_heavy_restore)
	ldr	x9, [x0, #TD_PCB]
	ldr	x10, [x9, #PCB_TTBR0]
	load_ttbr0 x10, x11
	ldr	x10, [x9, #PCB_TPIDR_EL0]
	msr	tpidr_el0, x10
	restore_switch_frame
END(cpu_heavy_restore)

/*
 * First code run by a thread created with cpu_set_thread_handler().
 * The stack holds { cpu_kthread_restore, exit function }; the thread
 * function and its argument are in the pcb.
 */
ENTRY(cpu_kthread_restore)
	mov	x20, x0
	mov	x29, #0
	kernel_ttbr0
	msr	daifclr, #3
	mov	x0, x19
	bl	lwkt_switch_return
	ldr	w9, [x20, #TD_CRITCOUNT]
	sub	w9, w9, #1
	str	w9, [x20, #TD_CRITCOUNT]
	ldr	x9, [x20, #TD_PCB]
	ldr	x0, [x9, #PCB_X20]	/* argument */
	ldr	x10, [x9, #PCB_X19]	/* thread function */
	ldr	x30, [sp, #8]		/* where it returns to */
	add	sp, sp, #16
	br	x10
END(cpu_kthread_restore)

/*
 * First code run by a cpu's idle thread; the stack holds only
 * { cpu_idle_restore, pad }.
 */
ENTRY(cpu_idle_restore)
	kernel_ttbr0
	add	sp, sp, #16
	mov	x29, #0
	mov	x30, #0
	msr	daifclr, #3
	mov	x0, x19
	bl	lwkt_switch_return
	b	cpu_idle
END(cpu_idle_restore)

/*
 * thread_t cpu_exit_switch(thread_t ntd)
 *
 * The switch function of a thread that is exiting: nothing is saved
 * because the thread will never be resumed.
 */
ENTRY(cpu_exit_switch)
	adrp	x9, arm64_ttbr0_kernel
	ldr	x9, [x9, :lo12:arm64_ttbr0_kernel]
	load_ttbr0 x9, x10
	msr	daifset, #3
	mrs	x11, tpidr_el1
	ldr	x19, [x11, #GD_CURTHREAD]
	str	x0, [x11, #GD_CURTHREAD]
	ldr	x12, [x0, #TD_SP]
	mov	sp, x12
	ldr	x10, [sp]
	br	x10
END(cpu_exit_switch)

/*
 * First code run by a forked thread, reached through cpu_heavy_restore
 * with x0 = old thread, x19 = function, x20 = argument and sp pointing
 * at the new lwp's trapframe.
 */
ENTRY(fork_trampoline)
	bl	lwkt_switch_return
	mrs	x9, tpidr_el1
	ldr	x9, [x9, #GD_CURTHREAD]
	ldr	w10, [x9, #TD_CRITCOUNT]
	sub	w10, w10, #1
	str	w10, [x9, #TD_CRITCOUNT]
	mov	x0, x20
	mov	x1, sp
	blr	x19
	/*
	 * The function returned, which means "go to user mode" with the
	 * trapframe at sp.  That path does not exist yet.
	 */
	mov	x0, sp
	bl	arm64_usermode_unimplemented
1:	wfi
	b	1b
END(fork_trampoline)

/*
 * void savectx(struct pcb *pcb)
 *
 * Record the callee-saved state of the caller, for crash dumps and
 * stopped cpus.
 */
ENTRY(savectx)
	stp	x19, x20, [x0, #(PCB_X19 + 0)]
	stp	x21, x22, [x0, #(PCB_X19 + 16)]
	stp	x23, x24, [x0, #(PCB_X19 + 32)]
	stp	x25, x26, [x0, #(PCB_X19 + 48)]
	stp	x27, x28, [x0, #(PCB_X19 + 64)]
	stp	x29, x30, [x0, #(PCB_X19 + 80)]
	mov	x9, sp
	str	x9, [x0, #(PCB_X19 + 96)]
	ret
END(savectx)
