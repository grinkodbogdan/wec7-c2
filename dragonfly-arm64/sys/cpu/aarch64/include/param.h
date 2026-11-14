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

#ifndef _CPU_PARAM_H_
#define _CPU_PARAM_H_

/*
 * Machine dependent constants for aarch64.
 */
#ifndef _MACHINE
#define	_MACHINE	aarch64
#endif
#ifndef _MACHINE_ARCH
#define	_MACHINE_ARCH	aarch64
#endif
#ifndef MACHINE
#define MACHINE		"aarch64"
#endif
#ifndef MACHINE_ARCH
#define	MACHINE_ARCH	"aarch64"
#endif

/*
 * Use SMP_MAXCPU instead of MAXCPU for structures that are intended to
 * remain compatible between UP and SMP builds.
 */
#define SMP_MAXCPU	256
#define MAXCPU		SMP_MAXCPU

/*
 * 4K translation granule, 4 levels, 48-bit virtual addresses.
 *
 * The x86 names (PDR/PDP/PML4) are kept as aliases for the equivalent
 * level of the aarch64 table walk because machine-independent code
 * (and pmap code shared in spirit with pc64) uses them:
 *
 *	x86 PML4 == L0 (512GB per entry)
 *	x86 PDP  == L1 (1GB per entry)
 *	x86 PD   == L2 (2MB per entry)
 *	x86 PT   == L3 (4KB per entry)
 */
#define NPTEPGSHIFT	9		/* LOG2(NPTEPG) */
#define PAGE_SHIFT	12		/* LOG2(PAGE_SIZE) */
#define PAGE_SIZE	(1<<PAGE_SHIFT)	/* bytes/page */
#define PAGE_MASK	(PAGE_SIZE-1)
#define NPTEPG		(PAGE_SIZE/8LU)	/* PAGE_SIZE/sizeof(pt_entry_t) */

#define L3_SHIFT	12
#define L2_SHIFT	21
#define L1_SHIFT	30
#define L0_SHIFT	39
#define L3_SIZE		(1UL << L3_SHIFT)
#define L2_SIZE		(1UL << L2_SHIFT)
#define L1_SIZE		(1UL << L1_SHIFT)
#define L0_SIZE		(1UL << L0_SHIFT)
#define Ln_ENTRIES	(1 << NPTEPGSHIFT)
#define Ln_ADDR_MASK	(Ln_ENTRIES - 1)

#define PDRSHIFT	L2_SHIFT	/* LOG2(NBPDR) */
#define NBPDR		(1<<PDRSHIFT)	/* bytes/page dir */
#define PDRMASK		(NBPDR-1)
#define NPDEPGSHIFT	9		/* LOG2(NPDEPG) */
#define NPDEPG		(PAGE_SIZE/8LU)	/* PAGE_SIZE/sizeof(pd_entry_t) */

#define PDPSHIFT	L1_SHIFT	/* LOG2(NBPDP) */
#define NBPDP		(1<<PDPSHIFT)	/* bytes/page dir ptr table */
#define PDPMASK		(NBPDP-1)
#define NPDPEPGSHIFT	9		/* LOG2(NPDPEPG) */
#define NPDPEPG		(PAGE_SIZE/8LU)	/* PAGE_SIZE/sizeof(pdp_entry_t) */

#define PML4SHIFT	L0_SHIFT	/* LOG2(NBPML4) */
#define NBPML4		(1UL<<PML4SHIFT)/* bytes/page map level4 table */
#define PML4MASK	(NBPML4-1)
#define NPML4EPGSHIFT	9		/* LOG2(NPML4EPG) */
#define NPML4EPG	(PAGE_SIZE/8LU)	/* PAGE_SIZE/sizeof(pml4_entry_t) */

/* bits 63:48 must all equal bit 47 with 48-bit VAs */
#define PML4_SIGNMASK  0xFFFF800000000000LU

/* bytes per mmu segment: one level 2 entry */
#define SEG_SHIFT	L2_SHIFT
#define SEG_SIZE	(1<<SEG_SHIFT)
#define SEG_MASK	(SEG_SIZE-1)

#define DEV_BSHIFT	9		/* log2(DEV_BSIZE) */
#define DEV_BSIZE	(1<<DEV_BSHIFT)
#define DEV_BMASK	(DEV_BSIZE - 1)

#ifndef BLKDEV_IOSIZE
#define BLKDEV_IOSIZE	PAGE_SIZE	/* default block device I/O size */
#endif
#define DFLTPHYS	(64 * 1024)	/* default max raw I/O transfer size */
#define MAXPHYS		(128 * 1024)	/* max raw I/O transfer size */
#define MAXDUMPPGS	(MAXPHYS/PAGE_SIZE)

#define UPAGES		4		/* pages of u-area */

/*
 * Ceiling on amount of swblock kva space, can be changed via
 * kern.maxswzone /boot/loader.conf variable.  On 64 bit machines
 * the kernel has a variable limit so set this to 0.
 */
#ifndef VM_SWZONE_SIZE_MAX
#define VM_SWZONE_SIZE_MAX	(256L * 1024L * 1024 * 1024)
#endif

/*
 * Ceiling on size of buffer cache (really only effects write queueing,
 * the VM page cache is not effected), can be changed via
 * kern.maxbcache /boot/loader.conf variable.
 */
#ifndef VM_BCACHE_SIZE_MAX
#define VM_BCACHE_SIZE_MAX	0
#endif

/*
 * Constants related to network buffer management.
 */

/* clicks to bytes */
#define ctob(x)	((x)<<PAGE_SHIFT)

/* bytes to clicks */
#define btoc(x)	(((unsigned)(x)+PAGE_MASK)>>PAGE_SHIFT)

/*
 * btodb() is messy and perhaps slow because `bytes' may be an off_t.  We
 * want to shift an unsigned type to avoid sign extension and we don't
 * want to widen `bytes' unnecessarily.  Assume that the result fits in
 * a daddr_t.
 */
#define btodb(bytes)	 		/* calculates (bytes / DEV_BSIZE) */ \
	(sizeof (bytes) > sizeof(long) \
	 ? (daddr_t)((unsigned long long)(bytes) >> DEV_BSHIFT) \
	 : (daddr_t)((unsigned long)(bytes) >> DEV_BSHIFT))

#define dbtob(db)			/* calculates (db * DEV_BSIZE) */ \
	((off_t)(db) << DEV_BSHIFT)

/*
 * Mach derived conversion macros
 */
#define	round_page(x)	((((unsigned long)(x)) + PAGE_MASK) & ~(unsigned long)(PAGE_MASK))
#define	trunc_page(x)	((unsigned long)(x) & ~(unsigned long)(PAGE_MASK))

#if defined(_KERNEL) || defined(_KERNEL_STRUCTURES)
#define	atop(x)		((vm_pindex_t)((x) >> PAGE_SHIFT))
#define	ptoa(x)		((vm_paddr_t)(x) << PAGE_SHIFT)

#define	pgtok(x)	((x) * (PAGE_SIZE / 1024))
#endif	/* _KERNEL || _KERNEL_STRUCTURES */

#ifdef _KERNEL
#define        _TSTMP(x)                       {}
#define        TSTMP(class, unit, event, par)  _TSTMP(0)
#endif /* _KERNEL */

#endif /* !_CPU_PARAM_H_ */
