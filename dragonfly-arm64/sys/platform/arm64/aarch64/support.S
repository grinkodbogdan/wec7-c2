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
 * Memory primitives and the user/kernel copy routines.
 *
 * These are simple byte loops: correct, small and slow.  They need
 * word-at-a-time versions (and DC ZVA for zeroing pages) before
 * performance matters.
 *
 * Every routine that touches a user address arms pcb_onfault first.
 * If the access faults and vm_fault() cannot resolve it, trap() resumes
 * at the recorded address instead of panicing, and the routine returns
 * its failure value.
 */

#include <cpu/asm.h>
#include "assym.s"

/* void *memset(void *dst, int c, size_t len) */
ENTRY(memset)
	mov	x3, x0
	cbz	x2, 2f
1:	strb	w1, [x3], #1
	subs	x2, x2, #1
	b.ne	1b
2:	ret
END(memset)

/* void bzero(void *dst, size_t len) */
ENTRY(bzero)
	cbz	x1, 2f
1:	strb	wzr, [x0], #1
	subs	x1, x1, #1
	b.ne	1b
2:	ret
END(bzero)

/* void pagezero(void *addr) */
ENTRY(pagezero)
	mov	x1, #4096
1:	stp	xzr, xzr, [x0], #16
	subs	x1, x1, #16
	b.ne	1b
	ret
END(pagezero)

/* void bcopy(const void *src, void *dst, size_t len) -- may overlap */
ENTRY(bcopy)
	mov	x3, x0
	mov	x0, x1
	mov	x1, x3
	b	memmove
END(bcopy)

/* void *memmove(void *dst, const void *src, size_t len) */
ENTRY(memmove)
	sub	x3, x0, x1
	cmp	x3, x2
	b.hs	memcpy			/* no destructive overlap */
	cbz	x2, 2f
	add	x3, x0, x2
	add	x1, x1, x2
1:	ldrb	w4, [x1, #-1]!
	strb	w4, [x3, #-1]!
	subs	x2, x2, #1
	b.ne	1b
2:	ret
END(memmove)

/* void *memcpy(void *dst, const void *src, size_t len) */
ENTRY(memcpy)
	mov	x3, x0
	cbz	x2, 2f
1:	ldrb	w4, [x1], #1
	strb	w4, [x3], #1
	subs	x2, x2, #1
	b.ne	1b
2:	ret
END(memcpy)

/* int bcmp(const void *b1, const void *b2, size_t len) -- 0 if equal */
ENTRY(bcmp)
	cbz	x2, 2f
1:	ldrb	w3, [x0], #1
	ldrb	w4, [x1], #1
	cmp	w3, w4
	b.ne	3f
	subs	x2, x2, #1
	b.ne	1b
2:	mov	x0, #0
	ret
3:	mov	x0, #1
	ret
END(bcmp)

/*
 * x9 = current pcb.  Clobbers x10.
 */
.macro	get_pcb
	mrs	x9, tpidr_el1
	ldr	x9, [x9, #GD_CURTHREAD]
	ldr	x9, [x9, #TD_PCB]
.endm

/* Arm pcb_onfault to resume at \label.  Expects x9 = pcb. */
.macro	set_onfault label
	adr	x10, \label
	str	x10, [x9, #PCB_ONFAULT]
	mov	x10, sp
	str	x10, [x9, #PCB_ONFAULT_SP]
.endm

.macro	clear_onfault
	str	xzr, [x9, #PCB_ONFAULT]
.endm

/*
 * Branch to \label unless [\addr, \addr + \len) lies entirely inside
 * the user address space.  Clobbers x10, x11.
 */
.macro	check_user addr, len, label
	adds	x10, \addr, \len
	b.cs	\label			/* wrapped */
	ldr	x11, =VM_MAX_USER_ADDRESS
	cmp	x10, x11
	b.hi	\label
.endm

/*
 * int copyin(const void *udaddr, void *kaddr, size_t len)
 * int copyout(const void *kaddr, void *udaddr, size_t len)
 */
ENTRY(copyin)
	check_user x0, x2, copy_efault
	b	copy_common
END(copyin)

ENTRY(copyout)
	check_user x1, x2, copy_efault
	/* FALLTHROUGH */
copy_common:
	get_pcb
	set_onfault copy_fault
	cbz	x2, 2f
1:	ldrb	w4, [x0], #1
	strb	w4, [x1], #1
	subs	x2, x2, #1
	b.ne	1b
2:	clear_onfault
	mov	x0, #0
	ret
copy_fault:
	get_pcb
	clear_onfault
copy_efault:
	mov	x0, #EFAULT
	ret
END(copyout)

/*
 * int copyinstr(const void *udaddr, void *kaddr, size_t len, size_t *done)
 *
 * Copy a NUL terminated string in from user space.  *done, if not
 * NULL, receives the number of bytes copied including the NUL.
 */
ENTRY(copyinstr)
	get_pcb
	set_onfault copystr_fault
	mov	x5, #0			/* bytes copied */
	ldr	x6, =VM_MAX_USER_ADDRESS
1:	cmp	x5, x2
	b.hs	3f			/* out of buffer */
	cmp	x0, x6
	b.hs	copystr_fault		/* ran off the user address space */
	ldrb	w4, [x0], #1
	strb	w4, [x1], #1
	add	x5, x5, #1
	cbnz	w4, 1b
	mov	x0, #0
2:	clear_onfault
	cbz	x3, 4f
	str	x5, [x3]
4:	ret
3:	mov	x0, #ENAMETOOLONG
	b	2b
copystr_fault:
	get_pcb
	mov	x0, #EFAULT
	mov	x5, #0
	b	2b
END(copyinstr)

/*
 * int copystr(const void *kfaddr, void *kdaddr, size_t len, size_t *done)
 *
 * As copyinstr, kernel to kernel.
 */
ENTRY(copystr)
	mov	x5, #0
1:	cmp	x5, x2
	b.hs	3f
	ldrb	w4, [x0], #1
	strb	w4, [x1], #1
	add	x5, x5, #1
	cbnz	w4, 1b
	mov	x0, #0
2:	cbz	x3, 4f
	str	x5, [x3]
4:	ret
3:	mov	x0, #ENAMETOOLONG
	b	2b
END(copystr)

/*
 * Fetch from and store to user memory.  The fetches return -1 on a
 * fault, which callers cannot tell from a stored -1; the stores return
 * 0 or -1.
 */
.macro	fetch name, size, insn, reg
ENTRY(\name)
	mov	x2, #\size
	check_user x0, x2, fusu_efault
	get_pcb
	set_onfault fusu_fault
	\insn	\reg, [x0]
	clear_onfault
	ret
END(\name)
.endm

.macro	store name, size, insn, reg
ENTRY(\name)
	mov	x2, #\size
	check_user x0, x2, fusu_efault
	get_pcb
	set_onfault fusu_fault
	\insn	\reg, [x0]
	clear_onfault
	mov	x0, #0
	ret
END(\name)
.endm

	fetch	fubyte, 1, ldrb, w0
	fetch	fuword32, 4, ldrsw, x0
	fetch	fuword64, 8, ldr, x0
	store	subyte, 1, strb, w1
	store	suword32, 4, str, w1
	store	suword64, 8, str, x1

ENTRY(fusu_fault)
	get_pcb
	clear_onfault
fusu_efault:
	mov	x0, #-1
	ret
END(fusu_fault)

/*
 * Atomic operations on user memory, used by the umtx/futex code.
 * LL/SC loops; a fault inside one simply abandons it.
 *
 * uint32_t swapu32(volatile uint32_t *p, uint32_t val)
 * uint64_t swapu64(volatile uint64_t *p, uint64_t val)
 * uint32_t fuwordadd32(volatile uint32_t *p, uint32_t val)
 * uint64_t fuwordadd64(volatile uint64_t *p, uint64_t val)
 * uint32_t casu32(volatile uint32_t *p, uint32_t old, uint32_t new)
 * uint64_t casu64(volatile uint64_t *p, uint64_t old, uint64_t new)
 *
 * swap and cas return the previous value, fuwordadd the previous value
 * as well (fetch-and-add), all of them -1 on a fault.
 */
.macro	uatomic name, size, r, body:vararg
ENTRY(\name)
	mov	x5, #\size
	check_user x0, x5, fusu_efault
	get_pcb
	set_onfault fusu_fault
1:	ldxr	\r\()4, [x0]
	\body
	stxr	w6, \r\()5, [x0]
	cbnz	w6, 1b
2:	clear_onfault
	mov	\r\()0, \r\()4
	ret
END(\name)
.endm

	uatomic	swapu32, 4, w, mov w5, w1
	uatomic	swapu64, 8, x, mov x5, x1
	uatomic	fuwordadd32, 4, w, add w5, w4, w1
	uatomic	fuwordadd64, 8, x, add x5, x4, x1

ENTRY(casu32)
	mov	x5, #4
	check_user x0, x5, fusu_efault
	get_pcb
	set_onfault fusu_fault
1:	ldxr	w4, [x0]
	cmp	w4, w1
	b.ne	2f
	stxr	w6, w2, [x0]
	cbnz	w6, 1b
2:	clear_onfault
	mov	w0, w4
	ret
END(casu32)

ENTRY(casu64)
	mov	x5, #8
	check_user x0, x5, fusu_efault
	get_pcb
	set_onfault fusu_fault
1:	ldxr	x4, [x0]
	cmp	x4, x1
	b.ne	2f
	stxr	w6, x2, [x0]
	cbnz	w6, 1b
2:	clear_onfault
	mov	x0, x4
	ret
END(casu64)
