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

#ifndef _MACHINE_PMAP_H_
#define	_MACHINE_PMAP_H_

#include <cpu/pmap.h>

/*
 * With a 4K granule and 48-bit VAs an aarch64 translation table has
 * exactly the shape of an x86_64 one.  The differences are in the PTE
 * bits and in there being two roots: TTBR0 for the user half and TTBR1
 * for the kernel half, so a user pmap holds no kernel entries at all.
 * See <machine/vmparam.h> for the layout.
 */
#define KVADDR(l4, l3, l2, l1) ( \
	((unsigned long)-1 << 47) | \
	((unsigned long)(l4) << PML4SHIFT) | \
	((unsigned long)(l3) << PDPSHIFT) | \
	((unsigned long)(l2) << PDRSHIFT) | \
	((unsigned long)(l1) << PAGE_SHIFT))

#define UVADDR(l4, l3, l2, l1) ( \
	((unsigned long)(l4) << PML4SHIFT) | \
	((unsigned long)(l3) << PDPSHIFT) | \
	((unsigned long)(l2) << PDRSHIFT) | \
	((unsigned long)(l1) << PAGE_SHIFT))

#define NUPDP_USER	(NPML4EPG/2)
#define NUPDP_TOTAL	(NPML4EPG)
#define NUPD_TOTAL	(NPDPEPG * NUPDP_TOTAL)
#define NUPT_TOTAL	(NPDEPG * NUPD_TOTAL)
#define NUPTE_TOTAL	((vm_pindex_t)NPTEPG * NUPT_TOTAL)
#define NUPTE_USER	((vm_pindex_t)NPTEPG * NPDEPG * NPDPEPG * NUPDP_USER)

#ifndef LOCORE

#ifndef _SYS_TYPES_H_
#include <sys/types.h>
#endif
#ifndef _SYS_CPUMASK_H_
#include <sys/cpumask.h>
#endif
#ifndef _SYS_QUEUE_H_
#include <sys/queue.h>
#endif
#ifndef _SYS_SPINLOCK_H_
#include <sys/spinlock.h>
#endif
#ifndef _SYS_THREAD_H_
#include <sys/thread.h>
#endif
#ifndef _MACHINE_TYPES_H_
#include <machine/types.h>
#endif
#ifndef _MACHINE_PARAM_H_
#include <machine/param.h>
#endif
#ifndef _MACHINE_VMPARAM_H_
#include <machine/vmparam.h>
#endif

struct pmap;
struct pv_entry;
struct vm_page;
struct vm_object;
struct vmspace;

/*
 * vm_page structure extension for pmap.
 */
struct md_page {
	TAILQ_HEAD(, pv_entry)	pv_list;	/* mappings of this page */
	int			pv_list_count;
	int			pt_refs;	/* valid entries, if a PT page */
};

#define MD_PAGE_FREEABLE(m)	\
	(((m)->flags & (PG_MAPPED | PG_WRITEABLE)) == 0)

/*
 * vm_object's representing large mappings can contain embedded pmaps
 * to organize sharing at higher page table levels for PROT_READ and
 * PROT_READ|PROT_WRITE maps.
 */
struct md_object {
	void *dummy_unused;
};

/*
 * Each machine dependent implementation is expected to
 * keep certain statistics.  They may do this anyway they
 * so choose, but are expected to return the statistics
 * in the following structure.
 */
struct pmap_statistics {
	long resident_count;    /* # of pages mapped (total) */
	long wired_count;       /* # of pages wired */
};
typedef struct pmap_statistics *pmap_statistics_t;

struct pmap {
	pml4_entry_t		*pm_pml4;	/* DMAP address of level 0 table */
	TAILQ_ENTRY(pmap)	pm_pmnode;	/* list of pmaps */
	int			pm_count;	/* reference count */
	cpulock_t		pm_active_lock; /* interlock */
	cpumask_t		pm_active;	/* active on cpus */
	int			pm_flags;
	struct pmap_statistics	pm_stats;	/* pmap statistics */
	struct spinlock		pm_spin;
	uint64_t		pm_ttbr0;	/* value for TTBR0_EL1 */
};

#define PMAP_FLAG_SIMPLE	0x00000001
#define PMAP_SEGSHARED		0x00000008	/* segment shared opt */
#define PMAP_MULTI		0x00000010	/* multi-threaded use */

#define pmap_resident_count(pmap)	\
	((pmap)->pm_stats.resident_count)
#define pmap_resident_tlnw_count(pmap)	\
	((pmap)->pm_stats.resident_count - (pmap)->pm_stats.wired_count)

typedef struct pmap	*pmap_t;

#ifdef _KERNEL
extern struct pmap	*kernel_pmap;
#endif

/*
 * For each managed vm_page_t there is a list of all currently valid
 * virtual mappings of that page, one pv_entry per mapping.
 */
typedef struct pv_entry {
	pmap_t		pv_pmap;	/* pmap where mapping lies */
	vm_offset_t	pv_va;		/* virtual address for mapping */
	TAILQ_ENTRY(pv_entry) pv_list;	/* on the page's list */
} *pv_entry_t;

#ifdef	_KERNEL

extern vm_paddr_t avail_end;
extern vm_paddr_t avail_start;
extern vm_offset_t clean_eva;
extern vm_offset_t clean_sva;
extern char *ptvmmap;		/* poor name! */

#ifndef __VM_PAGE_T_DEFINED__
#define __VM_PAGE_T_DEFINED__
typedef struct vm_page *vm_page_t;
#endif
#ifndef __VM_MEMATTR_T_DEFINED__
#define __VM_MEMATTR_T_DEFINED__
typedef char vm_memattr_t;
#endif

void	pmap_release(struct pmap *pmap);
void	pmap_interlock_wait (struct vmspace *);
void	pmap_bootstrap (vm_paddr_t *, vm_paddr_t);
void	arm64_sync_icache(vm_offset_t va, vm_size_t len);
void	*pmap_mapdev (vm_paddr_t, vm_size_t);
void	*pmap_mapdev_attr(vm_paddr_t, vm_size_t, int);
void	*pmap_mapdev_uncacheable(vm_paddr_t, vm_size_t);
void	pmap_page_set_memattr(vm_page_t m, vm_memattr_t ma);
void	pmap_unmapdev (vm_offset_t, vm_size_t);
void	pmap_invalidate_cache_pages(vm_page_t *pages, int count);
void	pmap_invalidate_cache_range(vm_offset_t sva, vm_offset_t eva);

/*
 * Software emulation of accessed/dirty bits is an x86 EPT concern that
 * vm_fault asks every pmap about.  Never needed here.
 */
static __inline int
pmap_emulate_ad_bits(pmap_t pmap __unused)
{
	return (0);
}

#endif /* _KERNEL */

#endif /* !LOCORE */

#endif /* !_MACHINE_PMAP_H_ */
