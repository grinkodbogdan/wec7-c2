/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <machine/asm.h>
#include <asmcontext.h>

/*
 * int getcontext(ucontext_t *ucp)
 *
 * Save the caller's context as if this function had just returned 1;
 * setcontext() (sigreturn) resumes it there.  The callee-saved
 * registers and the FP/SIMD state are saved; the caller-saved ones are
 * dead across the call anyway.
 */
	.weak	getcontext
	.set	getcontext,_getcontext
ENTRY(_getcontext)
	stp	x0, x30, [sp, #-16]!
	add	x2, x0, #UC_SIGMASK	/* oset */
	mov	x1, #0			/* set */
	mov	x0, #SIG_BLOCK
	bl	_sigprocmask
	ldp	x0, x30, [sp], #16

	str	xzr, [x0, #UC_LINK]
	add	x1, x0, #UC_MCONTEXT
	str	xzr, [x1, #MC_ONSTACK]
	mov	x2, sp
	str	x2, [x1, #MC_SP]
	str	x30, [x1, #MC_LR]
	str	x30, [x1, #MC_ELR]	/* resume at our return address */
	str	xzr, [x1, #MC_SPSR]	/* EL0t, flags clear */
	mrs	x2, tpidr_el0
	str	x2, [x1, #MC_TPIDR]

	add	x3, x1, #MC_X
	mov	x2, #1			/* getcontext() "returns" 1 */
	str	x2, [x3]
	stp	x19, x20, [x3, #(19 * 8)]
	stp	x21, x22, [x3, #(21 * 8)]
	stp	x23, x24, [x3, #(23 * 8)]
	stp	x25, x26, [x3, #(25 * 8)]
	stp	x27, x28, [x3, #(27 * 8)]
	str	x29, [x3, #(29 * 8)]

	/* FP/SIMD, in struct savefpu layout: q0-q31, fpsr, fpcr */
	add	x3, x1, #MC_FPREGS
	stp	q0, q1, [x3], #32
	stp	q2, q3, [x3], #32
	stp	q4, q5, [x3], #32
	stp	q6, q7, [x3], #32
	stp	q8, q9, [x3], #32
	stp	q10, q11, [x3], #32
	stp	q12, q13, [x3], #32
	stp	q14, q15, [x3], #32
	stp	q16, q17, [x3], #32
	stp	q18, q19, [x3], #32
	stp	q20, q21, [x3], #32
	stp	q22, q23, [x3], #32
	stp	q24, q25, [x3], #32
	stp	q26, q27, [x3], #32
	stp	q28, q29, [x3], #32
	stp	q30, q31, [x3], #32
	mrs	x4, fpsr
	mrs	x5, fpcr
	stp	w4, w5, [x3]

	ldr	w2, =_MC_FPFMT_VFP
	str	w2, [x1, #MC_FPFORMAT]
	ldr	w2, =_MC_FPOWNED_FPU
	str	w2, [x1, #MC_OWNEDFP]
	ldr	w2, =SIZEOF_MCONTEXT_T
	str	w2, [x1, #MC_LEN]

	mov	x0, #0
	ret
END(_getcontext)

	.section .note.GNU-stack,"",%progbits
