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
 * Physical map management for arm64.
 *
 * This is a deliberately simple pmap: correct first, fast later.
 *
 *   - Translation tables are always reached through the direct map, so
 *     there is no recursive mapping and no temporary mapping window.
 *   - Reverse mappings are a plain list of pv_entry per vm_page, as in
 *     the vkernel pmap, rather than pc64's backing-object scans.
 *   - One token serializes every operation.  pc64 uses fine grained
 *     per-pv locking; that work is still to be done here.
 *   - Address space ids are not used.  Switching TTBR0 flushes the TLB
 *     (see swtch.S), and user mappings are still marked not-global.
 *   - Invalidations use the broadcast (inner shareable) TLBI forms, so
 *     no IPI-based shootdown is needed even with multiple cpus.
 *
 * Accessed and modified tracking
 *
 *   ARMv8.0 hardware does not update accessed/dirty bits in page table
 *   entries.  Instead of emulating them with faults, this pmap keeps
 *   one invariant: a managed page with any writable mapping is dirty.
 *   Entering a writable mapping dirties the page at once, and the only
 *   way to clean a page is to write-protect it first (which is what
 *   pmap_clear_modify() does), so the next write faults and re-dirties
 *   it.  That is always safe and costs some unnecessary page-outs.
 *   Reference tracking is not emulated at all: every mapping is created
 *   accessed and pmap_ts_referenced() reports nothing, which makes the
 *   pageout daemon's choices less informed but never wrong.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/proc.h>
#include <sys/msgbuf.h>
#include <sys/vmmeter.h>
#include <sys/mman.h>
#include <sys/thread.h>

#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/vm_kern.h>
#include <vm/vm_page.h>
#include <vm/vm_map.h>
#include <vm/vm_object.h>
#include <vm/vm_extern.h>
#include <vm/vm_pageout.h>
#include <vm/vm_pager.h>
#include <vm/vm_zone.h>

#include <sys/thread2.h>
#include <sys/spinlock2.h>
#include <vm/vm_page2.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/pcb.h>
#include <machine/pmap.h>
#include <machine/vmparam.h>

#define	MINPV		2048
#ifndef PMAP_SHPGPERPROC
#define	PMAP_SHPGPERPROC	1000
#endif

#define	L0_IDX(va)	(((va) >> L0_SHIFT) & Ln_ADDR_MASK)
#define	L1_IDX(va)	(((va) >> L1_SHIFT) & Ln_ADDR_MASK)
#define	L2_IDX(va)	(((va) >> L2_SHIFT) & Ln_ADDR_MASK)
#define	L3_IDX(va)	(((va) >> L3_SHIFT) & Ln_ADDR_MASK)

#define	PTE_IS_VALID(pte)	(((pte) & ATTR_DESCR_VALID) != 0)
#define	PTE_IS_TABLE(pte)	(((pte) & ATTR_DESCR_MASK) == L1_TABLE)
#define	PTE_WRITABLE(pte)	(((pte) & ATTR_AP_RO) == 0)

/* Attributes common to every leaf entry this pmap creates. */
#define	PTE_LEAF_COMMON		(ATTR_AF | ATTR_SH_INNER)

struct pmap	kernel_pmap_store;
struct pmap	*kernel_pmap = &kernel_pmap_store;

vm_paddr_t	avail_start;		/* PA of first available physical page */
vm_paddr_t	avail_end;		/* PA of last available physical page */
vm_offset_t	virtual_start;		/* VA of first avail page */
vm_offset_t	virtual_end;		/* VA of last avail page */
vm_offset_t	virtual2_start;
vm_offset_t	virtual2_end;
vm_offset_t	KvaStart;		/* VA start of KVA space */
vm_offset_t	KvaEnd;			/* VA end of KVA space (non-inclusive) */
vm_offset_t	KvaSize;		/* max size of kernel virtual address space */
vm_offset_t	DMapMaxAddress;
vm_offset_t	kernel_vm_end;		/* end of KVA with page tables */
char		*ptvmmap;		/* one page of KVA for /dev/mem */
struct msgbuf	*msgbufp;

/* Value of TTBR0_EL1 while no user address space is installed. */
uint64_t	arm64_ttbr0_kernel;

/* pmap_token is provided by kern/lwkt_token.c */
static boolean_t pmap_initialized;

static vm_zone_t pvzone;
static struct vm_zone pvzone_store;
static struct pv_entry *pvinit;
static long	pv_entry_count;
static long	pv_entry_max;
static long	pv_entry_high_water;
static int	pmap_pagedaemon_waken;

/* The (always empty) level 0 tables for "no user space" and proc0. */
static pt_entry_t empty_l0[Ln_ENTRIES] __aligned(PAGE_SIZE);
static pt_entry_t proc0_l0[Ln_ENTRIES] __aligned(PAGE_SIZE);

static vm_paddr_t kernphys;		/* PA the kernel image is loaded at */
extern char	kernbase[];
extern pt_entry_t boot_l0_ttbr1[], boot_l1_dmap[];

static int	pmap_remove_pte(pmap_t pmap, pt_entry_t *pte, vm_offset_t va);

/*
 * TLB maintenance.  With no ASIDs in use, "all ASIDs" forms are right
 * for user and kernel addresses alike.
 */
static __inline void
tlb_inval_page(vm_offset_t va)
{
	uint64_t arg = (va >> PAGE_SHIFT) & 0xfffffffffffUL;

	__asm __volatile(
	    "dsb	ishst\n"
	    "tlbi	vaae1is, %0\n"
	    "dsb	ish\n"
	    "isb" : : "r" (arg) : "memory");
}

static __inline void
tlb_inval_all(void)
{
	__asm __volatile(
	    "dsb	ishst\n"
	    "tlbi	vmalle1is\n"
	    "dsb	ish\n"
	    "isb" : : : "memory");
}

/* Make a just-written translation table entry visible to the walker. */
static __inline void
pte_sync(void)
{
	__asm __volatile("dsb	ishst\n\tisb" : : : "memory");
}

static __inline pt_entry_t *
pte_to_table(pt_entry_t pte)
{
	return ((pt_entry_t *)PHYS_TO_DMAP(pte & PG_FRAME));
}

/* Physical address of something inside the kernel image. */
static __inline vm_paddr_t
kimage_to_phys(void *addr)
{
	return ((vm_offset_t)addr - (vm_offset_t)kernbase + kernphys);
}

/*
 * Make instructions just written to [va, va + len) executable.  The
 * instruction and data caches are separate, and unlike on x86 the
 * hardware does not keep them coherent.
 */
void
arm64_sync_icache(vm_offset_t va, vm_size_t len)
{
	uint64_t ctr = READ_SPECIALREG(ctr_el0);
	vm_offset_t dline = 4UL << ((ctr >> 16) & 0xf);
	vm_offset_t iline = 4UL << (ctr & 0xf);
	vm_offset_t eva = va + len;
	vm_offset_t p;

	for (p = va & ~(dline - 1); p < eva; p += dline)
		__asm __volatile("dc cvau, %0" : : "r" (p));
	__asm __volatile("dsb ish");
	for (p = va & ~(iline - 1); p < eva; p += iline)
		__asm __volatile("ic ivau, %0" : : "r" (p));
	__asm __volatile("dsb ish\n\tisb");
}

/*
 * Return a pointer to the level 3 entry for (va), or NULL if the
 * tables leading to it do not exist or (va) is covered by a block.
 */
static pt_entry_t *
pmap_pte(pmap_t pmap, vm_offset_t va)
{
	pt_entry_t pte;

	pte = pmap->pm_pml4[L0_IDX(va)];
	if (!PTE_IS_TABLE(pte))
		return (NULL);
	pte = pte_to_table(pte)[L1_IDX(va)];
	if (!PTE_IS_TABLE(pte))
		return (NULL);
	pte = pte_to_table(pte)[L2_IDX(va)];
	if (!PTE_IS_TABLE(pte))
		return (NULL);
	return (&pte_to_table(pte)[L3_IDX(va)]);
}

/*
 * Translate (va) in (pmap), understanding block mappings at every
 * level.  Returns 0 if nothing is mapped there.
 */
static vm_paddr_t
pmap_walk(pmap_t pmap, vm_offset_t va)
{
	pt_entry_t pte;

	pte = pmap->pm_pml4[L0_IDX(va)];
	if (!PTE_IS_TABLE(pte))
		return (0);
	pte = pte_to_table(pte)[L1_IDX(va)];
	if (!PTE_IS_VALID(pte))
		return (0);
	if (!PTE_IS_TABLE(pte))
		return ((pte & PG_FRAME & ~(L1_SIZE - 1)) | (va & (L1_SIZE - 1)));
	pte = pte_to_table(pte)[L2_IDX(va)];
	if (!PTE_IS_VALID(pte))
		return (0);
	if (!PTE_IS_TABLE(pte))
		return ((pte & PG_FRAME & ~(L2_SIZE - 1)) | (va & (L2_SIZE - 1)));
	pte = pte_to_table(pte)[L3_IDX(va)];
	if (!PTE_IS_VALID(pte))
		return (0);
	return ((pte & PG_FRAME) | (va & PAGE_MASK));
}

/*
 * Bootstrap
 */

/*
 * Carve zeroed pages off the front of free physical memory.  Only
 * usable before vm_page_startup().
 */
static vm_paddr_t
boot_alloc(vm_paddr_t *firstaddr, int npages)
{
	vm_paddr_t pa = *firstaddr;

	bzero((void *)PHYS_TO_DMAP(pa), (size_t)npages * PAGE_SIZE);
	*firstaddr += (vm_paddr_t)npages * PAGE_SIZE;
	return (pa);
}

/*
 * Remove [start, end) from the direct map.
 *
 * Memory the device tree marks "no-map" belongs to firmware (the
 * secure world, the modem).  It must not merely go unused: while it is
 * mapped as ordinary memory the cpu may read it speculatively, and on
 * some platforms a stray access to protected memory resets the machine.
 *
 * The direct map is made of 1GB blocks, so this splits the affected
 * ones into 2MB blocks and, at ragged edges, 4KB pages.  Each split
 * replaces a live block with a table that maps the same thing, which
 * has to go through an invalid entry (break-before-make); the new
 * table is therefore completed before the swap, and nothing in between
 * touches the direct map.  Returns the number of pages unmapped.
 */
static u_long
dmap_unmap(vm_paddr_t *firstaddr, vm_paddr_t start, vm_paddr_t end)
{
	pt_entry_t *l1e, *l2, *l2e, *l3, attrs;
	vm_paddr_t pa, tpa;
	u_long count = 0;
	int i;

	start = trunc_page(start);
	end = round_page(end);
	for (pa = start; pa < end; ) {
		if (pa >= Ln_ENTRIES * L1_SIZE)
			break;
		l1e = &boot_l1_dmap[L1_IDX(pa)];
		if (!PTE_IS_VALID(*l1e)) {
			pa = (pa + L1_SIZE) & ~(L1_SIZE - 1);
			continue;
		}
		if (!PTE_IS_TABLE(*l1e)) {
			/* 1GB block -> table of 512 2MB blocks */
			attrs = *l1e & ~(PG_FRAME | ATTR_DESCR_MASK);
			tpa = boot_alloc(firstaddr, 1);
			l2 = (pt_entry_t *)PHYS_TO_DMAP(tpa);
			for (i = 0; i < Ln_ENTRIES; ++i) {
				l2[i] = ((pa & ~(L1_SIZE - 1)) +
				    (vm_paddr_t)i * L2_SIZE) | L2_BLOCK | attrs;
			}
			*l1e = 0;
			tlb_inval_all();
			*l1e = tpa | L1_TABLE;
			pte_sync();
		}
		l2e = &pte_to_table(*l1e)[L2_IDX(pa)];
		if ((pa & (L2_SIZE - 1)) == 0 && end - pa >= L2_SIZE) {
			/* a whole 2MB entry goes */
			if (PTE_IS_VALID(*l2e)) {
				*l2e = 0;
				count += L2_SIZE / PAGE_SIZE;
			}
			pa += L2_SIZE;
			continue;
		}
		if (!PTE_IS_VALID(*l2e)) {
			pa = (pa + L2_SIZE) & ~(L2_SIZE - 1);
			continue;
		}
		if (!PTE_IS_TABLE(*l2e)) {
			/* 2MB block -> table of 512 pages */
			attrs = *l2e & ~(PG_FRAME | ATTR_DESCR_MASK);
			tpa = boot_alloc(firstaddr, 1);
			l3 = (pt_entry_t *)PHYS_TO_DMAP(tpa);
			for (i = 0; i < Ln_ENTRIES; ++i) {
				l3[i] = ((pa & ~(L2_SIZE - 1)) +
				    (vm_paddr_t)i * PAGE_SIZE) | L3_PAGE | attrs;
			}
			*l2e = 0;
			tlb_inval_all();
			*l2e = tpa | L2_TABLE;
			pte_sync();
		}
		l3 = pte_to_table(*l2e);
		if (PTE_IS_VALID(l3[L3_IDX(pa)])) {
			l3[L3_IDX(pa)] = 0;
			++count;
		}
		pa += PAGE_SIZE;
	}
	tlb_inval_all();
	return (count);
}

/* Pages of firmware-owned memory removed from the direct map. */
u_long	arm64_nomap_pages;

/*
 * Bootstrap the system enough to run with virtual memory.
 *
 * On entry the kernel runs on locore's tables: the kernel image is
 * mapped at KERNBASE and the low 4GB of physical memory through the
 * DMAP.  (*firstaddr) is the first free physical address after the
 * kernel and is advanced past everything allocated here.
 */
void
pmap_bootstrap(vm_paddr_t *firstaddr, vm_paddr_t kernel_pa)
{
	pt_entry_t *l0, *l1, *l2;
	vm_paddr_t maxpa = 4UL * L1_SIZE;
	vm_paddr_t pa;
	int i;

	kernphys = kernel_pa;

	/*
	 * Extend the direct map to cover all of physical memory with
	 * 1GB blocks.  Locore mapped [0, 4GB), with the first GB typed
	 * as device memory because that is where QEMU's virt machine
	 * and many SoCs keep their peripherals.
	 *
	 * XXX A machine with RAM in the first GB needs that entry split
	 * and its devices mapped with pmap_mapdev() instead.
	 */
	l1 = boot_l1_dmap;
	for (i = 0; i < arm64_bootinfo.bi_nmem; ++i) {
		vm_paddr_t start = arm64_bootinfo.bi_mem[i].mr_start;
		vm_paddr_t end = start + arm64_bootinfo.bi_mem[i].mr_size;

		if (end > DMAP_MAX_ADDRESS - DMAP_MIN_ADDRESS)
			end = DMAP_MAX_ADDRESS - DMAP_MIN_ADDRESS;
		if (end > Ln_ENTRIES * L1_SIZE)
			end = Ln_ENTRIES * L1_SIZE;	/* one L1 table */
		for (pa = start & ~(L1_SIZE - 1); pa < end; pa += L1_SIZE) {
			if (PTE_IS_VALID(l1[L1_IDX(pa)]))
				continue;
			l1[L1_IDX(pa)] = pa | L1_BLOCK | PTE_LEAF_COMMON |
			    ATTR_IDX(MAIR_IDX_NORMAL) | ATTR_UXN | ATTR_PXN;
		}
		if (end > maxpa)
			maxpa = roundup2(end, L1_SIZE);
	}
	pte_sync();
	DMapMaxAddress = DMAP_MIN_ADDRESS + maxpa;

	for (i = 0; i < arm64_bootinfo.bi_nresv; ++i) {
		if (!arm64_bootinfo.bi_resv_nomap[i])
			continue;
		arm64_nomap_pages += dmap_unmap(firstaddr,
		    arm64_bootinfo.bi_resv[i].mr_start,
		    arm64_bootinfo.bi_resv[i].mr_start +
		    arm64_bootinfo.bi_resv[i].mr_size);
	}

	/*
	 * The kernel pmap is locore's TTBR1 table.
	 */
	kernel_pmap->pm_pml4 =
	    (pt_entry_t *)PHYS_TO_DMAP(kimage_to_phys(boot_l0_ttbr1));
	kernel_pmap->pm_count = 1;
	CPUMASK_ASSALLONES(kernel_pmap->pm_active);
	spin_init(&kernel_pmap->pm_spin, "pmapbootstrap");

	/*
	 * No user address space yet: park TTBR0 on an empty table and
	 * drop locore's identity mapping.
	 */
	arm64_ttbr0_kernel = kimage_to_phys(empty_l0);
	WRITE_SPECIALREG(ttbr0_el1, arm64_ttbr0_kernel);
	__asm __volatile("isb");
	cpu_invltlb();

	/*
	 * Kernel virtual memory proper occupies one level 0 slot.  Give
	 * it its level 1 table and enough lower tables for the first
	 * 2MB; pmap_growkernel() adds the rest on demand once the VM
	 * system can allocate pages.
	 */
	KvaStart = VM_MIN_KERNEL_ADDRESS;
	KvaEnd = VM_MAX_KERNEL_ADDRESS;
	KvaSize = KvaEnd - KvaStart;

	l0 = kernel_pmap->pm_pml4;
	pa = boot_alloc(firstaddr, 1);
	l0[L0_IDX(KvaStart)] = pa | L0_TABLE;
	l1 = (pt_entry_t *)PHYS_TO_DMAP(pa);
	pa = boot_alloc(firstaddr, 1);
	l1[L1_IDX(KvaStart)] = pa | L1_TABLE;
	l2 = (pt_entry_t *)PHYS_TO_DMAP(pa);
	pa = boot_alloc(firstaddr, 1);
	l2[L2_IDX(KvaStart)] = pa | L2_TABLE;
	pte_sync();
	kernel_vm_end = KvaStart + L2_SIZE;

	virtual_start = KvaStart;
	virtual_end = KvaEnd;
	virtual2_start = 0;
	virtual2_end = 0;

	/* One page of KVA for /dev/mem to map arbitrary pages into. */
	ptvmmap = (char *)virtual_start;
	virtual_start += PAGE_SIZE;

	/* The message buffer lives in the direct map. */
	pa = boot_alloc(firstaddr, howmany(MSGBUF_SIZE, PAGE_SIZE));
	msgbufp = (struct msgbuf *)PHYS_TO_DMAP(pa);
}

/*
 * Initialize the pmap module once the VM system is up: set up the pool
 * pv entries come from.  Until this runs, mappings are unmanaged.
 */
void
pmap_init(void)
{
	vm_pindex_t initial_pvs;
	vm_pindex_t i;

	if (bootverbose)
		kprintf("boot: pmap_init, %ld pages\n", vm_page_array_size);

	/* vm_page_startup() does not run pmap_page_init() on these. */
	for (i = 0; i < vm_page_array_size; i++) {
		vm_page_t m = &vm_page_array[i];

		TAILQ_INIT(&m->md.pv_list);
		m->md.pv_list_count = 0;
		m->md.pt_refs = 0;
	}

	initial_pvs = vm_page_array_size;
	if (initial_pvs < MINPV)
		initial_pvs = MINPV;
	pvzone = &pvzone_store;
	pvinit = (struct pv_entry *)
	    kmem_alloc(kernel_map, initial_pvs * sizeof(struct pv_entry),
		       VM_SUBSYS_PVENTRY);
	zbootinit(pvzone, "PV ENTRY", sizeof(struct pv_entry), pvinit,
	    initial_pvs);
	pmap_initialized = TRUE;
	if (bootverbose)
		kprintf("boot: pmap_init done\n");
}

/*
 * Initialize the address space for the pv_entries.  Set a high water
 * mark so that the system can recover from excessive numbers of pv
 * entries.
 */
void
pmap_init2(void)
{
	long shpgperproc = PMAP_SHPGPERPROC;

	TUNABLE_LONG_FETCH("vm.pmap.shpgperproc", &shpgperproc);
	pv_entry_max = shpgperproc * maxproc + vm_page_array_size;
	TUNABLE_LONG_FETCH("vm.pmap.pv_entries", &pv_entry_max);
	pv_entry_high_water = 9 * (pv_entry_max / 10);
	zinitna(pvzone, NULL, 0, pv_entry_max, ZONE_INTERRUPT);
}

void
pmap_page_init(struct vm_page *m)
{
	vm_page_init(m);
	TAILQ_INIT(&m->md.pv_list);
	m->md.pv_list_count = 0;
	m->md.pt_refs = 0;
}

/*
 * Kernel-only primitives.  The caller owns the virtual address, the
 * page tables for it already exist (pmap_growkernel) and the mappings
 * are unmanaged.
 */

vm_paddr_t
pmap_kextract(vm_offset_t va)
{
	if (va >= DMAP_MIN_ADDRESS && va < DMapMaxAddress)
		return (DMAP_TO_PHYS(va));
	return (pmap_walk(kernel_pmap, va));
}

static __inline pt_entry_t
kernel_pte(vm_paddr_t pa)
{
	return (pa | L3_PAGE | PTE_LEAF_COMMON | ATTR_IDX(MAIR_IDX_NORMAL) |
	    ATTR_UXN | ATTR_PXN);
}

/*
 * Install (pte) for kernel address (va).  Returns non-zero if a valid
 * entry was replaced, in which case the old translation was also
 * removed from the TLB when (inval) is set.
 */
static int
pmap_kenter_pte(vm_offset_t va, pt_entry_t npte, int inval)
{
	pt_entry_t *pte = pmap_pte(kernel_pmap, va);
	pt_entry_t opte;

	KASSERT(pte != NULL, ("pmap_kenter: no page table for %p", (void *)va));
	opte = *pte;
	if (PTE_IS_VALID(opte)) {
		/* break before make */
		*pte = 0;
		if (inval)
			tlb_inval_page(va);
	}
	*pte = npte;
	pte_sync();
	return (PTE_IS_VALID(opte));
}

void
pmap_kenter(vm_offset_t va, vm_paddr_t pa)
{
	pmap_kenter_pte(va, kernel_pte(pa), 1);
}

/*
 * As pmap_kenter(), but the caller only needs the mapping to be right
 * on the current cpu.  Broadcast invalidation makes this identical.
 */
int
pmap_kenter_quick(vm_offset_t va, vm_paddr_t pa)
{
	return (pmap_kenter_pte(va, kernel_pte(pa), 1));
}

/*
 * As pmap_kenter(), but the caller takes care of TLB invalidation.
 * Returns non-zero if one is needed.
 */
int
pmap_kenter_noinval(vm_offset_t va, vm_paddr_t pa)
{
	return (pmap_kenter_pte(va, kernel_pte(pa), 0));
}

static void
pmap_kremove_pte(vm_offset_t va, int inval)
{
	pt_entry_t *pte = pmap_pte(kernel_pmap, va);

	if (pte == NULL || !PTE_IS_VALID(*pte))
		return;
	*pte = 0;
	if (inval)
		tlb_inval_page(va);
}

void
pmap_kremove(vm_offset_t va)
{
	pmap_kremove_pte(va, 1);
}

void
pmap_kremove_quick(vm_offset_t va)
{
	pmap_kremove_pte(va, 1);
}

void
pmap_kremove_noinval(vm_offset_t va)
{
	pmap_kremove_pte(va, 0);
}

/*
 * Used to map a range of physical addresses into kernel virtual
 * address space during early boot.  Everything is reachable through
 * the direct map, so no virtual space is consumed.
 */
vm_offset_t
pmap_map(vm_offset_t *virtp, vm_paddr_t start, vm_paddr_t end, int prot)
{
	if (bootverbose) {
		kprintf("boot: pmap_map %jx-%jx\n", (uintmax_t)start,
		    (uintmax_t)end);
	}
	return (PHYS_TO_DMAP(start));
}

void
pmap_qenter(vm_offset_t va, vm_page_t *m, int count)
{
	while (count-- > 0) {
		pmap_kenter_pte(va, kernel_pte(VM_PAGE_TO_PHYS(*m)), 1);
		va += PAGE_SIZE;
		m++;
	}
}

void
pmap_qenter_noinval(vm_offset_t va, vm_page_t *m, int count)
{
	while (count-- > 0) {
		pmap_kenter_pte(va, kernel_pte(VM_PAGE_TO_PHYS(*m)), 0);
		va += PAGE_SIZE;
		m++;
	}
}

void
pmap_qremove(vm_offset_t va, int count)
{
	while (count-- > 0) {
		pmap_kremove_pte(va, 1);
		va += PAGE_SIZE;
	}
}

void
pmap_qremove_quick(vm_offset_t va, int count)
{
	pmap_qremove(va, count);
}

void
pmap_qremove_noinval(vm_offset_t va, int count)
{
	while (count-- > 0) {
		pmap_kremove_pte(va, 0);
		va += PAGE_SIZE;
	}
}

struct vm_page *
pmap_kvtom(vm_offset_t va)
{
	return (PHYS_TO_VM_PAGE(pmap_kextract(va)));
}

/*
 * Map device memory into kernel virtual address space.
 */
void *
pmap_mapdev_attr(vm_paddr_t pa, vm_size_t size, int mode)
{
	vm_offset_t va, tmpva, offset;
	uint64_t idx;

	idx = (mode == PAT_WRITE_BACK) ? MAIR_IDX_NORMAL : MAIR_IDX_DEVICE;
	offset = pa & PAGE_MASK;
	size = roundup(offset + size, PAGE_SIZE);
	pa = trunc_page(pa);

	va = kmem_alloc_nofault(kernel_map, size, VM_SUBSYS_MAPDEV, PAGE_SIZE);
	if (va == 0)
		panic("pmap_mapdev: Couldn't alloc kernel virtual memory");
	for (tmpva = va; size > 0; size -= PAGE_SIZE) {
		pmap_kenter_pte(tmpva, pa | L3_PAGE | PTE_LEAF_COMMON |
		    ATTR_IDX(idx) | ATTR_UXN | ATTR_PXN, 1);
		tmpva += PAGE_SIZE;
		pa += PAGE_SIZE;
	}
	return ((void *)(va + offset));
}

void *
pmap_mapdev(vm_paddr_t pa, vm_size_t size)
{
	return (pmap_mapdev_attr(pa, size, PAT_UNCACHEABLE));
}

void *
pmap_mapdev_uncacheable(vm_paddr_t pa, vm_size_t size)
{
	return (pmap_mapdev_attr(pa, size, PAT_UNCACHEABLE));
}

void
pmap_unmapdev(vm_offset_t va, vm_size_t size)
{
	vm_offset_t base, offset;

	base = trunc_page(va);
	offset = va & PAGE_MASK;
	size = roundup(offset + size, PAGE_SIZE);
	pmap_qremove(base, size >> PAGE_SHIFT);
	kmem_free(kernel_map, base, size);
}

/*
 * Page table pages
 */

/*
 * Allocate a zeroed, wired page to use as a translation table.  May
 * block.
 */
static vm_page_t
pmap_alloc_ptpage(void)
{
	static u_int color;
	vm_page_t m;

	for (;;) {
		m = vm_page_alloc(NULL, color++, VM_ALLOC_NORMAL |
		    VM_ALLOC_SYSTEM | VM_ALLOC_INTERRUPT);
		if (m)
			break;
		vm_wait(0);
	}
	vm_page_wire(m);
	pmap_zero_page(VM_PAGE_TO_PHYS(m));
	m->valid = VM_PAGE_BITS_ALL;
	vm_page_flag_set(m, PG_UNQUEUED);
	m->md.pt_refs = 0;
	vm_page_wakeup(m);
	return (m);
}

static void
pmap_free_ptpage(vm_page_t m)
{
	vm_page_busy_wait(m, FALSE, "pmapfr");
	vm_page_unwire(m, 0);
	vm_page_free(m);
}

/*
 * Grow the page tables backing kernel virtual memory so that they
 * cover [kstart, kend).  Called by the VM system before it hands out
 * kernel virtual addresses.
 */
void
pmap_growkernel(vm_offset_t kstart, vm_offset_t kend)
{
	pt_entry_t *l1, *l2;
	vm_offset_t addr;
	vm_page_t m;

	kend = roundup2(kend, L2_SIZE);
	if (kend - 1 >= KvaEnd)
		kend = KvaEnd;

	lwkt_gettoken(&pmap_token);
	l1 = pte_to_table(kernel_pmap->pm_pml4[L0_IDX(KvaStart)]);
	for (addr = kernel_vm_end; addr < kend; addr += L2_SIZE) {
		if (!PTE_IS_VALID(l1[L1_IDX(addr)])) {
			m = pmap_alloc_ptpage();
			l1[L1_IDX(addr)] = VM_PAGE_TO_PHYS(m) | L1_TABLE;
		}
		l2 = pte_to_table(l1[L1_IDX(addr)]);
		if (!PTE_IS_VALID(l2[L2_IDX(addr)])) {
			m = pmap_alloc_ptpage();
			l2[L2_IDX(addr)] = VM_PAGE_TO_PHYS(m) | L2_TABLE;
		}
		pte_sync();
		kernel_vm_end = addr + L2_SIZE;
	}
	lwkt_reltoken(&pmap_token);
}

/*
 * Return a pointer to the level 3 entry for (va) in a user pmap,
 * creating the tables leading to it as needed.  May block.
 */
static pt_entry_t *
pmap_allocpte(pmap_t pmap, vm_offset_t va)
{
	pt_entry_t *table, *ptep;
	vm_page_t spare = NULL;
	vm_page_t parent;
	int level;

retry:
	table = pmap->pm_pml4;
	parent = NULL;
	for (level = 0; level < 3; ++level) {
		ptep = &table[(va >> (L0_SHIFT - level * NPTEPGSHIFT)) &
			      Ln_ADDR_MASK];
		if (!PTE_IS_VALID(*ptep)) {
			if (spare == NULL) {
				/*
				 * Allocating can block, and the tables can
				 * change while we do.  Walk again with the
				 * page in hand.
				 */
				spare = pmap_alloc_ptpage();
				goto retry;
			}
			*ptep = VM_PAGE_TO_PHYS(spare) | L0_TABLE;
			pte_sync();
			spare = NULL;
			if (parent)
				++parent->md.pt_refs;
			atomic_add_long(&pmap->pm_stats.wired_count, 1);
		}
		parent = PHYS_TO_VM_PAGE(*ptep & PG_FRAME);
		table = pte_to_table(*ptep);
	}
	if (spare)
		pmap_free_ptpage(spare);
	return (&table[L3_IDX(va)]);
}

/*
 * A level 3 entry of a user pmap at (va) went away.  Drop the
 * reference it held on its table page and free every table that
 * becomes empty as a result.
 */
static void
pmap_unuse_pt(pmap_t pmap, vm_offset_t va)
{
	pt_entry_t *ptep[3];
	vm_page_t m;
	int level;

	if (pmap == kernel_pmap)
		return;

	ptep[0] = &pmap->pm_pml4[L0_IDX(va)];
	ptep[1] = &pte_to_table(*ptep[0])[L1_IDX(va)];
	ptep[2] = &pte_to_table(*ptep[1])[L2_IDX(va)];

	for (level = 2; level >= 0; --level) {
		m = PHYS_TO_VM_PAGE(*ptep[level] & PG_FRAME);
		KKASSERT(m->md.pt_refs > 0);
		if (--m->md.pt_refs > 0)
			return;
		*ptep[level] = 0;
		/* The table may be cached in the walker; forget it. */
		tlb_inval_all();
		atomic_add_long(&pmap->pm_stats.wired_count, -1);
		pmap_free_ptpage(m);
	}
}

/*
 * pv entries
 */

static pv_entry_t
get_pv_entry(void)
{
	atomic_add_long(&pv_entry_count, 1);
	if (pv_entry_high_water &&
	    pv_entry_count > pv_entry_high_water &&
	    atomic_swap_int(&pmap_pagedaemon_waken, 1) == 0) {
		wakeup(&vm_pages_needed);
	}
	return (zalloc(pvzone));
}

static void
free_pv_entry(pv_entry_t pv)
{
	atomic_add_long(&pv_entry_count, -1);
	zfree(pvzone, pv);
}

static void
pmap_insert_entry(pmap_t pmap, vm_offset_t va, vm_page_t m)
{
	pv_entry_t pv = get_pv_entry();

	pv->pv_pmap = pmap;
	pv->pv_va = va;
	TAILQ_INSERT_TAIL(&m->md.pv_list, pv, pv_list);
	m->md.pv_list_count++;
	vm_page_flag_set(m, PG_MAPPED);
}

static void
pmap_remove_entry(pmap_t pmap, vm_page_t m, vm_offset_t va)
{
	pv_entry_t pv;

	TAILQ_FOREACH(pv, &m->md.pv_list, pv_list) {
		if (pv->pv_pmap == pmap && pv->pv_va == va)
			break;
	}
	KASSERT(pv != NULL, ("pmap_remove_entry: no pv for pmap %p va %p",
	    pmap, (void *)va));
	TAILQ_REMOVE(&m->md.pv_list, pv, pv_list);
	m->md.pv_list_count--;
	if (TAILQ_EMPTY(&m->md.pv_list))
		vm_page_flag_clear(m, PG_MAPPED | PG_WRITEABLE);
	free_pv_entry(pv);
}

/*
 * User-capable mapping operations
 */

/*
 * Tear down the valid mapping at (pte).  Returns non-zero if the page
 * table page holding it was freed as a result.
 */
static int
pmap_remove_pte(pmap_t pmap, pt_entry_t *pte, vm_offset_t va)
{
	pt_entry_t opte;
	vm_page_t m;

	opte = *pte;
	*pte = 0;
	tlb_inval_page(va);

	atomic_add_long(&pmap->pm_stats.resident_count, -1);
	if (opte & ATTR_SW_WIRED)
		atomic_add_long(&pmap->pm_stats.wired_count, -1);
	if (opte & ATTR_SW_MANAGED) {
		m = PHYS_TO_VM_PAGE(opte & PG_FRAME);
		if (PTE_WRITABLE(opte))
			vm_page_dirty(m);
		vm_page_flag_set(m, PG_REFERENCED);
		if (opte & ATTR_SW_WIRED)
			vm_page_unwire(m, -1);
		pmap_remove_entry(pmap, m, va);
	}
	pmap_unuse_pt(pmap, va);
	return (0);
}

/*
 * Insert the given physical page (m) at the specified virtual address
 * (va) in the target physical map with the protection requested.
 *
 * If specified, the page will be wired down, meaning that the related
 * pte can not be reclaimed.
 */
void
pmap_enter(pmap_t pmap, vm_offset_t va, vm_page_t m, vm_prot_t prot,
	   boolean_t wired, vm_map_entry_t entry __unused)
{
	pt_entry_t *pte, opte, npte;
	vm_paddr_t pa;
	uint64_t managed = 0;

	if (pmap == NULL)
		return;
	va = trunc_page(va);
	pa = VM_PAGE_TO_PHYS(m);

	lwkt_gettoken(&pmap_token);
	if (pmap == kernel_pmap) {
		pte = pmap_pte(pmap, va);
		KASSERT(pte != NULL,
		    ("pmap_enter: no kernel page table for %p", (void *)va));
	} else {
		pte = pmap_allocpte(pmap, va);
	}

	opte = *pte;
	if (PTE_IS_VALID(opte) && (opte & PG_FRAME) == pa) {
		/*
		 * Same page, the protection or wiring is changing.
		 */
		if (wired && (opte & ATTR_SW_WIRED) == 0) {
			atomic_add_long(&pmap->pm_stats.wired_count, 1);
			if (opte & ATTR_SW_MANAGED)
				vm_page_wire(m);
		} else if (!wired && (opte & ATTR_SW_WIRED)) {
			atomic_add_long(&pmap->pm_stats.wired_count, -1);
			if (opte & ATTR_SW_MANAGED)
				vm_page_unwire(m, -1);
		}
		managed = opte & ATTR_SW_MANAGED;
		*pte = 0;
		tlb_inval_page(va);
	} else {
		if (PTE_IS_VALID(opte)) {
			/*
			 * A different page is mapped here.  Hold the
			 * table page across the removal so it is not
			 * freed out from under us.
			 */
			vm_page_t ptm = PHYS_TO_VM_PAGE(
			    pmap_kextract((vm_offset_t)pte));

			if (pmap != kernel_pmap)
				++ptm->md.pt_refs;
			pmap_remove_pte(pmap, pte, va);
			if (pmap != kernel_pmap)
				--ptm->md.pt_refs;
		}
		if (pmap != kernel_pmap) {
			vm_page_t ptm = PHYS_TO_VM_PAGE(
			    pmap_kextract((vm_offset_t)pte));

			++ptm->md.pt_refs;
		}
		if (pmap_initialized &&
		    (m->flags & PG_FICTITIOUS) == 0) {
			pmap_insert_entry(pmap, va, m);
			managed = ATTR_SW_MANAGED;
		}
		atomic_add_long(&pmap->pm_stats.resident_count, 1);
		if (wired) {
			atomic_add_long(&pmap->pm_stats.wired_count, 1);
			if (managed)
				vm_page_wire(m);
		}
	}

	npte = pa | L3_PAGE | PTE_LEAF_COMMON | ATTR_IDX(MAIR_IDX_NORMAL) |
	    managed;
	if (wired)
		npte |= ATTR_SW_WIRED;
	if ((prot & VM_PROT_WRITE) == 0)
		npte |= ATTR_AP_RO;
	if (pmap == kernel_pmap) {
		npte |= ATTR_UXN;
		if ((prot & VM_PROT_EXECUTE) == 0)
			npte |= ATTR_PXN;
	} else {
		npte |= ATTR_AP_USER | ATTR_nG | ATTR_PXN;
		if ((prot & VM_PROT_EXECUTE) == 0)
			npte |= ATTR_UXN;
	}

	/* See "Accessed and modified tracking" at the top of this file. */
	if (managed && (prot & VM_PROT_WRITE)) {
		vm_page_flag_set(m, PG_WRITEABLE);
		vm_page_dirty(m);
	}
	if (prot & VM_PROT_EXECUTE)
		arm64_sync_icache(PHYS_TO_DMAP(pa), PAGE_SIZE);

	*pte = npte;
	pte_sync();
	lwkt_reltoken(&pmap_token);
}

/*
 * Advance (va) to the start of the next region a table entry at
 * (shift) covers.
 */
static __inline vm_offset_t
next_region(vm_offset_t va, int shift)
{
	return ((va + (1UL << shift)) & ~((1UL << shift) - 1));
}

/*
 * Apply (func) to every valid level 3 entry in [sva, eva) of (pmap).
 * (func) may remove the entry, including the table it lives in.
 */
static void
pmap_foreach_pte(pmap_t pmap, vm_offset_t sva, vm_offset_t eva,
		 void (*func)(pmap_t, pt_entry_t *, vm_offset_t, void *),
		 void *arg)
{
	vm_offset_t va = sva;
	pt_entry_t pte, *l3;

	while (va < eva) {
		pte = pmap->pm_pml4[L0_IDX(va)];
		if (!PTE_IS_TABLE(pte)) {
			va = next_region(va, L0_SHIFT);
			if (va == 0)
				break;
			continue;
		}
		pte = pte_to_table(pte)[L1_IDX(va)];
		if (!PTE_IS_TABLE(pte)) {
			va = next_region(va, L1_SHIFT);
			if (va == 0)
				break;
			continue;
		}
		pte = pte_to_table(pte)[L2_IDX(va)];
		if (!PTE_IS_TABLE(pte)) {
			va = next_region(va, L2_SHIFT);
			if (va == 0)
				break;
			continue;
		}
		l3 = &pte_to_table(pte)[L3_IDX(va)];
		if (PTE_IS_VALID(*l3))
			func(pmap, l3, va, arg);
		va += PAGE_SIZE;
		if (va == 0)
			break;
	}
}

static void
pmap_remove_callback(pmap_t pmap, pt_entry_t *pte, vm_offset_t va,
		     void *arg __unused)
{
	pmap_remove_pte(pmap, pte, va);
}

/*
 * Remove the given range of addresses from the specified map.
 */
void
pmap_remove(pmap_t pmap, vm_offset_t sva, vm_offset_t eva)
{
	if (pmap == NULL)
		return;
	lwkt_gettoken(&pmap_token);
	if (pmap->pm_stats.resident_count != 0)
		pmap_foreach_pte(pmap, sva, eva, pmap_remove_callback, NULL);
	lwkt_reltoken(&pmap_token);
}

/*
 * Remove every mapping in the range; called when an address space is
 * being torn down.
 */
void
pmap_remove_pages(pmap_t pmap, vm_offset_t sva, vm_offset_t eva)
{
	pmap_remove(pmap, sva, eva);
}

static void
pmap_protect_callback(pmap_t pmap, pt_entry_t *pte, vm_offset_t va,
		      void *arg __unused)
{
	pt_entry_t opte = *pte;

	if (!PTE_WRITABLE(opte))
		return;
	if (opte & ATTR_SW_MANAGED)
		vm_page_dirty(PHYS_TO_VM_PAGE(opte & PG_FRAME));
	*pte = 0;
	tlb_inval_page(va);
	*pte = opte | ATTR_AP_RO;
	pte_sync();
}

/*
 * Set the physical protection on the specified range of this map as
 * requested.  Only ever reduces access.
 */
void
pmap_protect(pmap_t pmap, vm_offset_t sva, vm_offset_t eva, vm_prot_t prot)
{
	if (pmap == NULL)
		return;
	if ((prot & (VM_PROT_READ | VM_PROT_EXECUTE)) == VM_PROT_NONE) {
		pmap_remove(pmap, sva, eva);
		return;
	}
	if (prot & VM_PROT_WRITE)
		return;
	lwkt_gettoken(&pmap_token);
	pmap_foreach_pte(pmap, sva, eva, pmap_protect_callback, NULL);
	lwkt_reltoken(&pmap_token);
}

/*
 * Removes this physical page from all physical maps in which it
 * resides.
 */
static void
pmap_remove_all(vm_page_t m)
{
	pv_entry_t pv;
	pt_entry_t *pte;

	lwkt_gettoken(&pmap_token);
	while ((pv = TAILQ_FIRST(&m->md.pv_list)) != NULL) {
		pte = pmap_pte(pv->pv_pmap, pv->pv_va);
		KKASSERT(pte != NULL && PTE_IS_VALID(*pte));
		pmap_remove_pte(pv->pv_pmap, pte, pv->pv_va);
	}
	lwkt_reltoken(&pmap_token);
}

/*
 * Write-protect every mapping of (m).
 */
static void
pmap_write_protect_page(vm_page_t m)
{
	pv_entry_t pv;
	pt_entry_t *pte;

	lwkt_gettoken(&pmap_token);
	TAILQ_FOREACH(pv, &m->md.pv_list, pv_list) {
		pte = pmap_pte(pv->pv_pmap, pv->pv_va);
		KKASSERT(pte != NULL && PTE_IS_VALID(*pte));
		pmap_protect_callback(pv->pv_pmap, pte, pv->pv_va, NULL);
	}
	vm_page_flag_clear(m, PG_WRITEABLE);
	lwkt_reltoken(&pmap_token);
}

/*
 * Lower the permission for all mappings to a given page.
 */
void
pmap_page_protect(vm_page_t m, vm_prot_t prot)
{
	if (!pmap_initialized || (m->flags & PG_FICTITIOUS))
		return;
	if ((prot & VM_PROT_WRITE) == 0) {
		if (prot & (VM_PROT_READ | VM_PROT_EXECUTE))
			pmap_write_protect_page(m);
		else
			pmap_remove_all(m);
	}
}

/*
 * Remove the mapping of (m) in (pmap), if any.
 */
void
pmap_remove_specific(pmap_t pmap, vm_page_t m)
{
	pv_entry_t pv, npv;
	pt_entry_t *pte;

	lwkt_gettoken(&pmap_token);
	TAILQ_FOREACH_MUTABLE(pv, &m->md.pv_list, pv_list, npv) {
		if (pv->pv_pmap != pmap)
			continue;
		pte = pmap_pte(pv->pv_pmap, pv->pv_va);
		KKASSERT(pte != NULL && PTE_IS_VALID(*pte));
		pmap_remove_pte(pv->pv_pmap, pte, pv->pv_va);
	}
	lwkt_reltoken(&pmap_token);
}

/*
 * A page with a writable mapping may have been written to.
 */
boolean_t
pmap_is_modified(vm_page_t m)
{
	return ((m->flags & PG_WRITEABLE) != 0);
}

/*
 * Forget that (m) was modified.  To keep later writes from going
 * unnoticed every mapping has to become read-only.
 */
void
pmap_clear_modify(vm_page_t m)
{
	if (!pmap_initialized || (m->flags & PG_FICTITIOUS))
		return;
	if (m->flags & PG_WRITEABLE)
		pmap_write_protect_page(m);
}

void
pmap_clear_reference(vm_page_t m __unused)
{
}

int
pmap_ts_referenced(vm_page_t m __unused)
{
	return (0);
}

/*
 * Change the wiring attribute for a map/virtual-address pair.  The
 * mapping must already exist in the pmap.  Returns the page if the
 * mapping was wired, for the caller to unwire.
 */
vm_page_t
pmap_unwire(pmap_t pmap, vm_offset_t *pva)
{
	vm_offset_t va = *pva;
	pt_entry_t *pte;
	vm_page_t m = NULL;

	*pva += PAGE_SIZE;
	if (pmap == NULL)
		return (NULL);
	lwkt_gettoken(&pmap_token);
	pte = pmap_pte(pmap, va);
	if (pte != NULL && PTE_IS_VALID(*pte) && (*pte & ATTR_SW_WIRED)) {
		atomic_add_long(&pmap->pm_stats.wired_count, -1);
		*pte &= ~ATTR_SW_WIRED;
		m = PHYS_TO_VM_PAGE(*pte & PG_FRAME);
	}
	lwkt_reltoken(&pmap_token);
	return (m);
}

/*
 * Extract the physical page address associated with the map/VA pair.
 * No page is held and no lock is returned, so (*handlep) is always
 * NULL and pmap_extract_done() has nothing to do.
 */
vm_paddr_t
pmap_extract(pmap_t pmap, vm_offset_t va, void **handlep)
{
	vm_paddr_t pa;

	if (handlep)
		*handlep = NULL;
	if (pmap == kernel_pmap)
		return (pmap_kextract(va));
	lwkt_gettoken(&pmap_token);
	pa = pmap_walk(pmap, va);
	lwkt_reltoken(&pmap_token);
	return (pa);
}

void
pmap_extract_done(void *handle __unused)
{
}

vm_paddr_t
uservtophys(vm_offset_t va)
{
	struct proc *p = curproc;

	if (p == NULL || va >= VM_MAX_USER_ADDRESS)
		return ((vm_paddr_t)-1);
	return (pmap_extract(vmspace_pmap(p->p_vmspace), va, NULL) ?:
	    (vm_paddr_t)-1);
}

/*
 * Optional fast path for vm_fault; decline and let it do the lookup.
 */
vm_page_t
pmap_fault_page_quick(pmap_t pmap __unused, vm_offset_t va __unused,
		      vm_prot_t prot __unused, int *busyp __unused)
{
	return (NULL);
}

/*
 * Address space lifecycle
 */

/*
 * Initialize pmap0/vmspace0, the address space of the kernel's own
 * process.  It never holds user mappings.
 */
void
pmap_pinit0(struct pmap *pmap)
{
	pmap->pm_pml4 = (pt_entry_t *)PHYS_TO_DMAP(kimage_to_phys(proc0_l0));
	pmap->pm_ttbr0 = kimage_to_phys(proc0_l0);
	pmap->pm_count = 1;
	CPUMASK_ASSZERO(pmap->pm_active);
	bzero(&pmap->pm_stats, sizeof(pmap->pm_stats));
	spin_init(&pmap->pm_spin, "pmapinit0");
}

/*
 * Initialize a preallocated and zeroed pmap structure, such as one in
 * a vmspace structure.
 */
void
pmap_pinit(struct pmap *pmap)
{
	vm_page_t m;

	m = pmap_alloc_ptpage();
	pmap->pm_pml4 = (pt_entry_t *)PHYS_TO_DMAP(VM_PAGE_TO_PHYS(m));
	pmap->pm_ttbr0 = VM_PAGE_TO_PHYS(m);
	pmap->pm_count = 1;
	CPUMASK_ASSZERO(pmap->pm_active);
	bzero(&pmap->pm_stats, sizeof(pmap->pm_stats));
	pmap->pm_stats.wired_count = 1;		/* the level 0 table */
	spin_init(&pmap->pm_spin, "pmapinit");
}

void
pmap_pinit2(struct pmap *pmap __unused)
{
}

/*
 * Clean up a pmap structure so it can be physically freed.  All of
 * its mappings and lower tables are gone by now (pmap_release()).
 */
void
pmap_puninit(pmap_t pmap)
{
	if (pmap->pm_pml4 != NULL) {
		pmap_free_ptpage(PHYS_TO_VM_PAGE(pmap->pm_ttbr0));
		pmap->pm_pml4 = NULL;
		pmap->pm_ttbr0 = 0;
		atomic_add_long(&pmap->pm_stats.wired_count, -1);
	}
}

/*
 * Release any resources held by the given physical map.  Called when
 * a pmap initialized by pmap_pinit is being released.
 */
void
pmap_release(struct pmap *pmap)
{
	KKASSERT(pmap != kernel_pmap);
	lwkt_gettoken(&pmap_token);
	/* Normally a no-op: vmspace teardown removed everything. */
	if (pmap->pm_stats.resident_count != 0) {
		pmap_foreach_pte(pmap, VM_MIN_USER_ADDRESS,
		    VM_MAX_USER_ADDRESS, pmap_remove_callback, NULL);
	}
	KASSERT(pmap->pm_stats.wired_count == 1,
	    ("pmap_release: %p still has %ld table pages", pmap,
	    pmap->pm_stats.wired_count - 1));
	lwkt_reltoken(&pmap_token);
}

void
pmap_reference(pmap_t pmap)
{
	if (pmap)
		atomic_add_int(&pmap->pm_count, 1);
}

void
pmap_init_proc(struct proc *p __unused)
{
}

/*
 * Lay out a new thread's kernel stack: the pcb sits at the very top
 * and the initial stack pointer just below it.
 */
void
pmap_init_thread(thread_t td)
{
	td->td_pcb = (struct pcb *)(td->td_kstack + td->td_kstack_size) - 1;
	td->td_savefpu = &td->td_pcb->pcb_save;
	td->td_sp = (char *)td->td_pcb;
}

void
pmap_interlock_wait(struct vmspace *vm __unused)
{
}

/*
 * Replace p->p_vmspace with a new one.  If adjrefs is non-zero the new
 * vmspace is referenced and the old one released.  Used by exec.
 */
void
pmap_replacevm(struct proc *p, struct vmspace *newvm, int adjrefs)
{
	struct vmspace *oldvm;
	struct lwp *lp;

	oldvm = p->p_vmspace;
	if (oldvm != newvm) {
		if (adjrefs)
			vmspace_ref(newvm);
		p->p_vmspace = newvm;
		KKASSERT(p->p_nthreads == 1);
		lp = RB_ROOT(&p->p_lwp_tree);
		pmap_setlwpvm(lp, newvm);
		if (adjrefs)
			vmspace_rel(oldvm);
	}
}

/*
 * Set the vmspace for a LWP.  The vmspace is almost universally set to
 * proc->p_vmspace, but virtual kernels need to swap it out for each
 * thread.
 */
void
pmap_setlwpvm(struct lwp *lp, struct vmspace *newvm)
{
	struct vmspace *oldvm;
	struct pmap *pmap;

	oldvm = lp->lwp_vmspace;
	if (oldvm == newvm)
		return;
	crit_enter();
	lp->lwp_vmspace = newvm;
	pmap = vmspace_pmap(newvm);
	lp->lwp_thread->td_pcb->pcb_ttbr0 = pmap->pm_ttbr0;
	if (curthread->td_lwp == lp) {
		ATOMIC_CPUMASK_ORBIT(pmap->pm_active, mycpu->gd_cpuid);
		WRITE_SPECIALREG(ttbr0_el1, pmap->pm_ttbr0);
		__asm __volatile("isb");
		cpu_invltlb();
		pmap = vmspace_pmap(oldvm);
		ATOMIC_CPUMASK_NANDBIT(pmap->pm_active, mycpu->gd_cpuid);
	}
	crit_exit();
}

/*
 * Page contents
 */

void
pmap_zero_page(vm_paddr_t phys)
{
	pagezero((void *)PHYS_TO_DMAP(phys));
}

void
pmap_zero_page_area(vm_paddr_t phys, int off, int size)
{
	bzero((char *)PHYS_TO_DMAP(phys) + off, size);
}

void
pmap_copy_page(vm_paddr_t src, vm_paddr_t dst)
{
	bcopy((void *)PHYS_TO_DMAP(src), (void *)PHYS_TO_DMAP(dst), PAGE_SIZE);
}

void
pmap_copy_page_frag(vm_paddr_t src, vm_paddr_t dst, size_t bytes)
{
	bcopy((char *)PHYS_TO_DMAP(src) + (src & PAGE_MASK),
	      (char *)PHYS_TO_DMAP(dst) + (dst & PAGE_MASK), bytes);
}

/*
 * Hints, statistics and optional operations
 */

void
pmap_invalidate_range(pmap_t pmap __unused, vm_offset_t sva, vm_offset_t eva)
{
	vm_offset_t va;

	if (eva - sva > 64 * PAGE_SIZE) {
		tlb_inval_all();
		return;
	}
	for (va = trunc_page(sva); va < eva; va += PAGE_SIZE)
		tlb_inval_page(va);
}

void
pmap_invalidate_cache_pages(vm_page_t *pages __unused, int count __unused)
{
}

void
pmap_invalidate_cache_range(vm_offset_t sva __unused, vm_offset_t eva __unused)
{
}

/* Copying mappings at fork is an optimization; faults will do. */
void
pmap_copy(pmap_t dst_pmap __unused, pmap_t src_pmap __unused,
	  vm_offset_t dst_addr __unused, vm_size_t len __unused,
	  vm_offset_t src_addr __unused)
{
}

/* Pre-faulting an object's pages is an optimization too. */
void
pmap_object_init_pt(pmap_t pmap __unused, struct vm_map_entry *entry __unused,
		    vm_offset_t addr __unused, vm_size_t size __unused,
		    int limit __unused)
{
}

int
pmap_prefault_ok(pmap_t pmap __unused, vm_offset_t addr __unused)
{
	return (0);
}

void
pmap_collect(void)
{
	pmap_pagedaemon_waken = 0;
}

void
pmap_pgscan(struct pmap_pgscan_info *pginfo __unused)
{
}

int
pmap_mincore(pmap_t pmap, vm_offset_t addr)
{
	vm_paddr_t pa = pmap_extract(pmap, addr, NULL);

	if (pa == 0)
		return (0);
	return (MINCORE_INCORE | MINCORE_REFERENCED | MINCORE_MODIFIED);
}

vm_paddr_t
pmap_phys_address(vm_pindex_t ppn)
{
	return (ptoa(ppn));
}

vm_offset_t
pmap_addr_hint(vm_object_t obj, vm_offset_t addr, vm_size_t size)
{
	if (obj == NULL || size < L2_SIZE || obj->type != OBJT_DEVICE)
		return (addr);
	return (roundup2(addr, L2_SIZE));
}

void
pmap_page_set_memattr(vm_page_t m __unused, vm_memattr_t ma __unused)
{
}

void
pmap_maybethreaded(pmap_t pmap __unused)
{
}

int
pmap_mapped_sync(vm_page_t m)
{
	return (m->flags);
}

void
pmap_object_init(vm_object_t object __unused)
{
}

void
pmap_object_free(vm_object_t object __unused)
{
}
