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
 * arm64 kernel entry.
 *
 * Entered per the Linux arm64 boot protocol:
 *	x0 = physical address of the flattened device tree
 *	MMU off, D-cache off, interrupts masked, EL2 or EL1
 *
 * We drop to EL1 if needed, build the bootstrap page tables, turn the MMU
 * on and continue in the upper half at KERNBASE.
 *
 * Bootstrap address space (4K granule, 48-bit VA, 4 levels):
 *
 *	TTBR0  0x0000000000000000  identity map of the low 4GB (1GB blocks),
 *				   only live until initarm() disables it
 *	TTBR1  0xfffff80000000000  DMAP, direct map of the low 4GB of
 *				   physical memory (same base as pc64)
 *	TTBR1  0xffffffff80000000  KERNBASE, 1GB of 2MB blocks starting at
 *				   the kernel's physical load address
 */

#define	L0_SHIFT	39
#define	L1_SHIFT	30
#define	L2_SHIFT	21
#define	PT_INDEX(va, s)	(((va) >> (s)) & 0x1ff)

#define	KERNBASE	0xffffffff80000000
#define	DMAP_BASE	0xfffff80000000000

#define	PTE_TABLE	0x3
#define	PTE_BLOCK	0x1
#define	PTE_AF		(1 << 10)
#define	PTE_SH_INNER	(3 << 8)
#define	PTE_ATTR_NORMAL	(0 << 2)		/* MAIR index 0 */
#define	PTE_ATTR_DEVICE	(1 << 2)		/* MAIR index 1 */
#define	PTE_UXN		(1 << 54)
#define	PTE_PXN		(1 << 53)

#define	BLK_NORMAL	(PTE_BLOCK | PTE_AF | PTE_SH_INNER | PTE_ATTR_NORMAL)
#define	BLK_DEVICE	(PTE_BLOCK | PTE_AF | PTE_ATTR_DEVICE)

/* MAIR: idx0 = normal write-back, idx1 = device nGnRE */
#define	MAIR_VALUE	0x04ff

/* T0SZ=T1SZ=16, 4K granules, WBWA inner-shareable walks */
#define	TCR_VALUE	((16) | (16 << 16) | \
			 (1 << 8) | (1 << 10) | (3 << 12) | \
			 (1 << 24) | (1 << 26) | (3 << 28) | (2 << 30))

/* RES1 bits + M, C, SA, I */
#define	SCTLR_VALUE	0x30d0180d

#define	BOOTSTACK_SIZE	(4 * 4096)

	.section .text.boot, "ax"
	.globl	_start
_start:
	/* Linux arm64 Image header */
	b	1f			/* code0 */
	.long	0			/* code1 */
	.quad	0			/* text_offset */
	.quad	_end - _start		/* image_size */
	.quad	0xa			/* flags: LE, 4K pages, load anywhere */
	.quad	0
	.quad	0
	.quad	0
	.ascii	"ARM\x64"		/* magic */
	.long	0

1:	mov	x19, x0			/* x19 = FDT physical address */
	msr	daifset, #0xf

	mrs	x1, CurrentEL
	lsr	x1, x1, #2
	cmp	x1, #2
	b.ne	in_el1

	/*
	 * Entered in EL2: configure the minimum needed for a 64-bit EL1
	 * with access to the generic timer and FP, then drop to EL1h.
	 */
	mov	x1, #(1 << 31)		/* HCR_EL2.RW: EL1 is AArch64 */
	msr	hcr_el2, x1
	mov	x1, #0x33ff		/* no CPTR traps */
	msr	cptr_el2, x1
	msr	hstr_el2, xzr
	mrs	x1, cnthctl_el2
	orr	x1, x1, #3		/* EL1 may use the physical timer */
	msr	cnthctl_el2, x1
	msr	cntvoff_el2, xzr
	mov	x1, #0x3c5		/* EL1h, DAIF masked */
	msr	spsr_el2, x1
	adr	x1, in_el1
	msr	elr_el2, x1
	eret

in_el1:
	msr	spsel, #1
	adr	x20, _start		/* x20 = kernel physical base */

	/* Zero the BSS; the bootstrap page tables and stack live in it. */
	adrp	x1, __bss_start
	add	x1, x1, :lo12:__bss_start
	adrp	x2, __bss_end
	add	x2, x2, :lo12:__bss_end
2:	cmp	x1, x2
	b.hs	3f
	str	xzr, [x1], #8
	b	2b
3:
	/* TTBR0: identity map 0-4GB.  First GB is device space. */
	adrp	x21, boot_l0_ttbr0
	adrp	x1, boot_l1_ident
	orr	x2, x1, #PTE_TABLE
	str	x2, [x21]
	ldr	x2, =(BLK_DEVICE | PTE_UXN | PTE_PXN)
	str	x2, [x1, #0]
	ldr	x2, =(BLK_NORMAL | PTE_UXN)
	mov	x3, #1
4:	orr	x4, x2, x3, lsl #L1_SHIFT
	str	x4, [x1, x3, lsl #3]
	add	x3, x3, #1
	cmp	x3, #4
	b.lo	4b

	/* TTBR1: DMAP of 0-4GB at DMAP_BASE. */
	adrp	x22, boot_l0_ttbr1
	adrp	x1, boot_l1_dmap
	orr	x2, x1, #PTE_TABLE
	str	x2, [x22, #(PT_INDEX(DMAP_BASE, L0_SHIFT) * 8)]
	ldr	x2, =(BLK_DEVICE | PTE_UXN | PTE_PXN)
	str	x2, [x1, #0]
	ldr	x2, =(BLK_NORMAL | PTE_UXN | PTE_PXN)
	mov	x3, #1
5:	orr	x4, x2, x3, lsl #L1_SHIFT
	str	x4, [x1, x3, lsl #3]
	add	x3, x3, #1
	cmp	x3, #4
	b.lo	5b

	/* TTBR1: kernel image, 1GB of 2MB blocks at KERNBASE. */
	adrp	x1, boot_l1_kern
	orr	x2, x1, #PTE_TABLE
	str	x2, [x22, #(PT_INDEX(KERNBASE, L0_SHIFT) * 8)]
	adrp	x3, boot_l2_kern
	orr	x2, x3, #PTE_TABLE
	str	x2, [x1, #(PT_INDEX(KERNBASE, L1_SHIFT) * 8)]
	ldr	x2, =(BLK_NORMAL | PTE_UXN)
	orr	x2, x2, x20		/* load address must be 2MB aligned */
	mov	x4, #0
6:	str	x2, [x3, x4, lsl #3]
	add	x2, x2, #(1 << L2_SHIFT)
	add	x4, x4, #1
	cmp	x4, #512
	b.lo	6b

	/* Program the translation regime and turn the MMU on. */
	ldr	x1, =MAIR_VALUE
	msr	mair_el1, x1
	ldr	x1, =TCR_VALUE
	mrs	x2, id_aa64mmfr0_el1	/* IPS = PARange, capped at 48 bits */
	and	x2, x2, #0x7
	cmp	x2, #5
	mov	x3, #5
	csel	x2, x2, x3, lo
	orr	x1, x1, x2, lsl #32
	msr	tcr_el1, x1
	msr	ttbr0_el1, x21
	msr	ttbr1_el1, x22
	isb
	tlbi	vmalle1
	dsb	nsh
	isb
	ldr	x1, =SCTLR_VALUE
	msr	sctlr_el1, x1
	isb

	/* Leave the identity mapping for the upper half. */
	ldr	x1, =virtstart
	br	x1

	.ltorg

	.text
virtstart:
	ldr	x1, =bootstack_top
	mov	sp, x1
	mov	x29, #0
	mov	x30, #0
	ldr	x1, =exception_vectors
	msr	vbar_el1, x1
	isb

	mov	x0, x19			/* FDT physical address */
	mov	x1, x20			/* kernel physical base */
	bl	initarm

7:	wfi
	b	7b

	.ltorg

	.bss
	.balign	4096
boot_l0_ttbr0:	.space	4096
boot_l1_ident:	.space	4096
	.globl	boot_l0_ttbr1
boot_l0_ttbr1:	.space	4096
boot_l1_dmap:	.space	4096
boot_l1_kern:	.space	4096
boot_l2_kern:	.space	4096
bootstack:	.space	BOOTSTACK_SIZE
bootstack_top:
