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
 *	TTBR1  0xffffffff80000000  KERNBASE, 32MB of 4KB pages starting
 *				   at the kernel's physical load address
 *
 * The kernel image is mapped with 4KB pages rather than 2MB blocks so
 * that it can be loaded at any page-aligned address.  Bootloaders that
 * predate the "load anywhere" flag of the Image header (the LK-based
 * ones on many Qualcomm phones, for example) put the kernel at a fixed
 * offset such as RAM base + 0x80000, which is not 2MB aligned.
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
#define	MAIR_VALUE	0x4404ff	/* 0 normal WB, 1 device nGnRE, 2 normal NC */

/* T0SZ=T1SZ=16, 4K granules, WBWA inner-shareable walks */
#define	TCR_VALUE	((16) | (16 << 16) | \
			 (1 << 8) | (1 << 10) | (3 << 12) | \
			 (1 << 24) | (1 << 26) | (3 << 28) | (2 << 30))

/* RES1 bits + M, C, SA, I */
#define	SCTLR_VALUE	0x30d0180d

#define	PAGE_NORMAL	(0x3 | PTE_AF | PTE_SH_INNER | PTE_ATTR_NORMAL)

/* Level 3 tables for the kernel image: 16 x 2MB = 32MB. */
#define	KERN_L3_TABLES	16

/*
 * Offset from the start of RAM at which a loader that honours the
 * Image header places the kernel.  Zero is fine for those; setting it
 * (-DTEXT_OFFSET=0x80000) exists to test the unaligned-load path.
 */
#ifndef TEXT_OFFSET
#define	TEXT_OFFSET	0
#endif

#define	BOOTSTACK_SIZE	(4 * 4096)

#include "assym.s"

	.section .text.boot, "ax"
	.globl	_start
_start:
	/* Linux arm64 Image header */
	b	1f			/* code0 */
	.long	0			/* code1 */
	.quad	TEXT_OFFSET		/* text_offset */
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
	/*
	 * With a GICv3 the cpu interface is a set of system registers,
	 * which EL2 must allow EL1 to use (ICC_SRE_EL2.Enable and .SRE).
	 */
	mrs	x1, id_aa64pfr0_el1
	ubfx	x1, x1, #24, #4
	cbz	x1, 8f
	mrs	x1, S3_4_C12_C9_5	/* ICC_SRE_EL2 */
	orr	x1, x1, #0x1		/* SRE */
	orr	x1, x1, #0x8		/* Enable */
	msr	S3_4_C12_C9_5, x1
	isb
8:
	mov	x1, #0x3c5		/* EL1h, DAIF masked */
	msr	spsr_el2, x1
	adr	x1, in_el1
	msr	elr_el2, x1
	eret

in_el1:
	msr	spsel, #1
	/*
	 * Bootloaders can leave the virtual timer armed.  An expired timer
	 * is a pending interrupt waiting for the moment interrupts are
	 * unmasked, so switch it off now.  The physical timer is left
	 * alone: a hypervisor may trap EL1's access to it (Qualcomm's
	 * does, on the Snapdragon 820), and the interrupt controller setup
	 * masks its interrupt anyway.
	 */
	msr	cntv_ctl_el0, xzr
	/*
	 * Do not trap FP/SIMD use.  The kernel itself is built without
	 * FP; this is for user programs, whose state swtch.S saves and
	 * restores.
	 */
	mov	x1, #(3 << 20)		/* CPACR_EL1.FPEN */
	msr	cpacr_el1, x1
	isb
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

	/* TTBR1: kernel image, 32MB of 4KB pages at KERNBASE. */
	adrp	x1, boot_l1_kern
	orr	x2, x1, #PTE_TABLE
	str	x2, [x22, #(PT_INDEX(KERNBASE, L0_SHIFT) * 8)]
	adrp	x3, boot_l2_kern
	orr	x2, x3, #PTE_TABLE
	str	x2, [x1, #(PT_INDEX(KERNBASE, L1_SHIFT) * 8)]
	adrp	x5, boot_l3_kern
	mov	x4, #0
6:	add	x2, x5, x4, lsl #12	/* level 2 entry -> its level 3 table */
	orr	x2, x2, #PTE_TABLE
	str	x2, [x3, x4, lsl #3]
	add	x4, x4, #1
	cmp	x4, #KERN_L3_TABLES
	b.lo	6b
	ldr	x2, =(PAGE_NORMAL | PTE_UXN)
	orr	x2, x2, x20		/* load address is page aligned */
	mov	x4, #0
7:	str	x2, [x5, x4, lsl #3]
	add	x2, x2, #4096
	add	x4, x4, #1
	cmp	x4, #(KERN_L3_TABLES * 512)
	b.lo	7b

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

	/*
	 * initarm() returns the top of thread0's kernel stack.  Move
	 * onto it and start the machine-independent kernel, which does
	 * not return.
	 */
	mov	sp, x0
	mov	x29, #0
	bl	mi_startup

7:	wfi
	b	7b

	.ltorg

/*
 * Entry point of a secondary cpu, started by PSCI CPU_ON with the MMU
 * off and x0 = its logical cpu number.  The page tables already exist,
 * so this is the boot cpu's path without the table building: drop to
 * EL1, turn the MMU on using locore's identity map, continue in the
 * upper half on the middle of the cpu's idle stack and call
 * init_secondary().  It then "resumes" its idle thread exactly as a
 * context switch would (see cpu_idle_restore in swtch.S).
 */
	.section .text.boot, "ax"
	.globl	mpentry
mpentry:
	mov	x19, x0			/* cpu number */
	msr	daifset, #0xf

	mrs	x1, CurrentEL
	lsr	x1, x1, #2
	cmp	x1, #2
	b.ne	1f
	mov	x1, #(1 << 31)
	msr	hcr_el2, x1
	mov	x1, #0x33ff
	msr	cptr_el2, x1
	msr	hstr_el2, xzr
	mrs	x1, cnthctl_el2
	orr	x1, x1, #3
	msr	cnthctl_el2, x1
	msr	cntvoff_el2, xzr
	mrs	x1, id_aa64pfr0_el1
	ubfx	x1, x1, #24, #4
	cbz	x1, 8f
	mrs	x1, S3_4_C12_C9_5	/* ICC_SRE_EL2 */
	orr	x1, x1, #0x1
	orr	x1, x1, #0x8
	msr	S3_4_C12_C9_5, x1
	isb
8:	mov	x1, #0x3c5
	msr	spsr_el2, x1
	adr	x1, 1f
	msr	elr_el2, x1
	eret
1:
	msr	spsel, #1
	mov	x1, #(3 << 20)		/* CPACR_EL1.FPEN */
	msr	cpacr_el1, x1
	isb

	ldr	x1, =MAIR_VALUE
	msr	mair_el1, x1
	ldr	x1, =TCR_VALUE
	mrs	x2, id_aa64mmfr0_el1
	and	x2, x2, #0x7
	cmp	x2, #5
	mov	x3, #5
	csel	x2, x2, x3, lo
	orr	x1, x1, x2, lsl #32
	msr	tcr_el1, x1
	adrp	x21, boot_l0_ttbr0
	adrp	x22, boot_l0_ttbr1
	msr	ttbr0_el1, x21
	msr	ttbr1_el1, x22
	isb
	/*
	 * Start from clean TLBs and instruction cache: a cpu powered up
	 * by firmware may hold whatever it last ran (as the iOS port to
	 * this chip found).
	 */
	tlbi	vmalle1
	ic	iallu
	dsb	sy
	isb
	ldr	x1, =SCTLR_VALUE
	msr	sctlr_el1, x1
	isb
	ldr	x1, =mp_virtstart
	br	x1

	.ltorg

	.text
mp_virtstart:
	ldr	x1, =exception_vectors
	msr	vbar_el1, x1
	ldr	x1, =CPU_prvspace
	ldr	x20, [x1, x19, lsl #3]	/* this cpu's privatespace */
	msr	tpidr_el1, x20		/* == its globaldata */
	add	x1, x20, #PS_BOOTSTACK_OFF
	mov	sp, x1
	mov	x29, #0
	isb

	mov	x0, x19
	bl	init_secondary

	/* Become the idle thread: x0 = x19 = it, sp = its saved sp. */
	ldr	x0, [x20, #GD_CURTHREAD]
	mov	x19, x0
	ldr	x1, [x0, #TD_SP]
	mov	sp, x1
	ldr	x10, [sp]
	br	x10

	.ltorg

	.bss
	.balign	4096
	.globl	boot_l0_ttbr0
boot_l0_ttbr0:	.space	4096
boot_l1_ident:	.space	4096
	.globl	boot_l0_ttbr1, boot_l1_dmap
boot_l0_ttbr1:	.space	4096
boot_l1_dmap:	.space	4096
boot_l1_kern:	.space	4096
boot_l2_kern:	.space	4096
boot_l3_kern:	.space	4096 * KERN_L3_TABLES
bootstack:	.space	BOOTSTACK_SIZE
bootstack_top:
