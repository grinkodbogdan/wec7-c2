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
 * Memory primitives.  On pc64 these live in support.s next to the
 * copyin/copyout family, which will join them here once there is a
 * userland to copy from.
 *
 * These are simple byte loops: correct, small and slow.  They need to
 * be replaced with word-at-a-time versions (and DC ZVA for bzero of
 * whole pages) before performance matters.
 */

	.text

/* void *memset(void *dst, int c, size_t len) */
	.globl	memset
	.type	memset, %function
memset:
	mov	x3, x0
	cbz	x2, 2f
1:	strb	w1, [x3], #1
	subs	x2, x2, #1
	b.ne	1b
2:	ret

/* void bzero(void *dst, size_t len) */
	.globl	bzero
	.type	bzero, %function
bzero:
	cbz	x1, 2f
1:	strb	wzr, [x0], #1
	subs	x1, x1, #1
	b.ne	1b
2:	ret

/* void bcopy(const void *src, void *dst, size_t len) -- may overlap */
	.globl	bcopy
	.type	bcopy, %function
bcopy:
	mov	x3, x0
	mov	x0, x1
	mov	x1, x3
	/* FALLTHROUGH */

/* void *memmove(void *dst, const void *src, size_t len) */
	.globl	memmove
	.type	memmove, %function
memmove:
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

/* void *memcpy(void *dst, const void *src, size_t len) */
	.globl	memcpy
	.type	memcpy, %function
memcpy:
	mov	x3, x0
	cbz	x2, 2f
1:	ldrb	w4, [x1], #1
	strb	w4, [x3], #1
	subs	x2, x2, #1
	b.ne	1b
2:	ret
