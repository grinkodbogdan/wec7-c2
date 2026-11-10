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

#ifndef _CPU_PMAP_H_
#define _CPU_PMAP_H_

/*
 * aarch64 stage 1 translation table entries, 4K granule.
 */
#ifndef LOCORE
#include <machine/stdint.h>

/*
 * The x86 type names are kept for each level of the walk so pmap code
 * can stay structurally close to pc64's (see <cpu/param.h>).
 */
typedef	__uint64_t	pml4_entry_t;	/* level 0 */
typedef	__uint64_t	pdp_entry_t;	/* level 1 */
typedef	__uint64_t	pd_entry_t;	/* level 2 */
typedef	__uint64_t	pt_entry_t;	/* level 3 */
#endif /* !LOCORE */

/* Descriptor type, bits 1:0 */
#define	ATTR_DESCR_VALID	0x1UL
#define	ATTR_DESCR_MASK		0x3UL
#define	L0_TABLE		0x3UL
#define	L1_BLOCK		0x1UL	/* 1GB */
#define	L1_TABLE		0x3UL
#define	L2_BLOCK		0x1UL	/* 2MB */
#define	L2_TABLE		0x3UL
#define	L3_PAGE			0x3UL	/* 4KB */

/* Lower attributes of block and page descriptors */
#define	ATTR_IDX(x)		((__uint64_t)(x) << 2)	/* MAIR index */
#define	ATTR_IDX_MASK		(7UL << 2)
#define	ATTR_NS			(1UL << 5)
#define	ATTR_AP_USER		(1UL << 6)	/* AP[1]: EL0 access */
#define	ATTR_AP_RO		(1UL << 7)	/* AP[2]: read-only */
#define	ATTR_SH_INNER		(3UL << 8)
#define	ATTR_AF			(1UL << 10)	/* access flag */
#define	ATTR_nG			(1UL << 11)	/* not global (ASID tagged) */

/* Upper attributes */
#define	ATTR_DBM		(1UL << 51)	/* hw dirty bit management */
#define	ATTR_CONTIGUOUS		(1UL << 52)
#define	ATTR_PXN		(1UL << 53)	/* privileged no-execute */
#define	ATTR_UXN		(1UL << 54)	/* user no-execute */
#define	ATTR_SW_WIRED		(1UL << 55)	/* software: wired */
#define	ATTR_SW_MANAGED		(1UL << 56)	/* software: managed */
#define	ATTR_SW_DIRTY		(1UL << 57)	/* software: modified */

/* MAIR_EL1 attribute indexes programmed by locore */
#define	MAIR_IDX_NORMAL		0	/* write-back cacheable */
#define	MAIR_IDX_DEVICE		1	/* device nGnRE */

#define	PG_FRAME	(0x0000fffffffff000UL)
#define	PG_PS_FRAME	(0x0000ffffffe00000UL)

/*
 * Page fault flags handed to vm_fault-facing code.  These are software
 * values derived from ESR_EL1; the names match x86_64's.
 */
#define PGEX_P		0x01	/* Protection violation vs. not present */
#define PGEX_W		0x02	/* during a Write cycle */
#define PGEX_U		0x04	/* access from User mode (UPL) */
#define PGEX_I		0x10	/* during an instruction fetch */

#endif /* !_CPU_PMAP_H_ */
