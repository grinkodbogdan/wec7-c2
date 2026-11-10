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
 * arm64 platform machine-dependent startup and miscellany.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/proc.h>
#include <sys/buf.h>
#include <sys/reboot.h>
#include <sys/msgbuf.h>
#include <sys/cons.h>
#include <sys/eventhandler.h>
#include <sys/random.h>
#include <sys/sysctl.h>

#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/vm_kern.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>
#include <vm/vm_map.h>
#include <vm/vm_pager.h>
#include <vm/vm_extern.h>

#include <sys/thread2.h>

#include <machine/cpu.h>
#include <machine/cpufunc.h>
#include <machine/globaldata.h>
#include <machine/md_var.h>
#include <machine/pcb.h>
#include <machine/pmap.h>
#include <machine/vmparam.h>

#define	PSCI_SYSTEM_OFF		0x84000008UL
#define	PSCI_SYSTEM_RESET	0x84000009UL

/* Fallback when the device tree names no UART: QEMU virt's PL011. */
#define	FALLBACK_UART		0x09000000UL

#define	PHYS_AVAIL_ENTRIES	(2 * ARM64_MAXMEMREGIONS + 4)

#define	MAXBUFSTRUCTSIZE	((size_t)512 * 1024 * 1024)

struct user;

extern char kernbase[], _end[];

struct arm64_bootinfo arm64_bootinfo;

/* The boot cpu's private data; other cpus' are allocated when started. */
static struct privatespace CPU_prvspace_bsp __aligned(PAGE_SIZE);
struct privatespace *CPU_prvspace[MAXCPU] = { &CPU_prvspace_bsp };

/* thread0's kernel stack, with its struct pcb at the top. */
static char proc0paddr_buff[LWKT_THREAD_STACK] __aligned(16);
static struct trapframe proc0_tf;

long		physmem;
vm_paddr_t	Maxmem;
vm_phystable_t	phys_avail[PHYS_AVAIL_ENTRIES];
vm_phystable_t	dump_avail[PHYS_AVAIL_ENTRIES];
static vm_paddr_t Realmem;

static vm_offset_t buffer_sva, buffer_eva;
vm_offset_t	clean_sva, clean_eva;
static vm_offset_t pager_sva, pager_eva;

uint64_t	*vm_page_dump;
vm_offset_t	vm_page_dump_size;
int		busdma_swi_pending;
void		(*cpu_idle_hook)(void);

/* Environment handed to kgetenv(), built from the boot arguments. */
static char	bootenv[1024];

SYSCTL_ULONG(_hw, OID_AUTO, realmem, CTLFLAG_RD, &Realmem, 0,
    "Physical memory reported by the firmware, in bytes");

static void	cpu_startup(void *);
SYSINIT(cpu, SI_BOOT2_START_CPU, SI_ORDER_FIRST, cpu_startup, NULL);

/*
 * Turn the boot arguments into boot flags and a kernel environment.
 * Words starting with '-' are flags as boot(8) knows them (-s single
 * user, -v verbose, -a ask for the root device); words of the form
 * name=value become environment variables, i.e. loader tunables.
 */
static void
parse_bootargs(const char *args)
{
	char *ep = bootenv;
	char *eend = bootenv + sizeof(bootenv) - 2;
	const char *p = args;

	while (p != NULL && *p != '\0') {
		const char *word;
		int has_eq = 0;

		while (*p == ' ')
			++p;
		word = p;
		while (*p != '\0' && *p != ' ') {
			if (*p == '=')
				has_eq = 1;
			++p;
		}
		if (word[0] == '-') {
			for (++word; word < p; ++word) {
				switch (*word) {
				case 'a': boothowto |= RB_ASKNAME; break;
				case 's': boothowto |= RB_SINGLE; break;
				case 'v': boothowto |= RB_VERBOSE; break;
				}
			}
		} else if (has_eq && ep + (p - word) < eend) {
			bcopy(word, ep, p - word);
			ep += p - word;
			*ep++ = '\0';
		}
	}
	*ep = '\0';
	kern_envp = bootenv;
	if (boothowto & RB_VERBOSE)
		bootverbose = 1;
}

/*
 * Add [start, end) to phys_avail[], leaving out any part that
 * overlaps one of the (nres) reserved ranges.
 */
static void
add_phys_avail(vm_paddr_t start, vm_paddr_t end,
	       const struct arm64_memregion *res, int nres, int *idx)
{
	int i;

	start = round_page(start);
	end = trunc_page(end);
	if (start >= end)
		return;
	for (i = 0; i < nres; ++i) {
		vm_paddr_t rs = trunc_page(res[i].mr_start);
		vm_paddr_t re = round_page(res[i].mr_start + res[i].mr_size);

		if (rs < end && re > start) {
			add_phys_avail(start, rs, res + i + 1, nres - i - 1,
			    idx);
			add_phys_avail(re, end, res + i + 1, nres - i - 1,
			    idx);
			return;
		}
	}
	if (*idx >= PHYS_AVAIL_ENTRIES - 1)
		return;
	phys_avail[*idx].phys_beg = start;
	phys_avail[*idx].phys_end = end;
	++*idx;
	physmem += atop(end - start);
	if (atop(end) > Maxmem)
		Maxmem = atop(end);
}

/*
 * C entry point, reached from locore.S with the MMU on and the kernel
 * running at KERNBASE on a temporary stack.  This is the counterpart
 * of pc64's hammer_time(): it sets up everything mi_startup() takes
 * for granted (per-cpu data, thread0, the pmap, the console, the list
 * of free physical memory) and returns the stack to run it on.
 *
 *	fdt_pa		physical address of the flattened device tree
 *	kernphys	physical address the kernel image was loaded at
 */
u_long
initarm(vm_paddr_t fdt_pa, vm_paddr_t kernphys)
{
	struct arm64_bootinfo *bi = &arm64_bootinfo;
	struct arm64_memregion reserved[2];
	struct mdglobaldata *gd;
	vm_paddr_t firstaddr;
	int fdt_ok;
	int i, n;

	/*
	 * Per-cpu data comes first: curthread and mycpu are used by
	 * nearly everything, including kprintf().
	 */
	gd = &CPU_prvspace[0]->mdglobaldata;
	WRITE_SPECIALREG(tpidr_el1, gd);
	gd->mi.gd_curthread = &thread0;
	thread0.td_gd = &gd->mi;

	/*
	 * The device tree lives in RAM somewhere above the kernel; reach
	 * it through the bootstrap direct map.
	 */
	bi->bi_timer_irq = 27;
	fdt_ok = (fdt_scan((void *)PHYS_TO_DMAP(fdt_pa), bi) == 0);
	cninit_early(bi->bi_uart_base ? bi->bi_uart_base : FALLBACK_UART);
	if (!fdt_ok || bi->bi_nmem == 0) {
		cnputs_early("\nDragonFly/arm64: no usable device tree, "
		    "cannot determine memory layout\n");
		for (;;)
			halt();
	}
	parse_bootargs(bi->bi_bootargs);

	/*
	 * Default to one cpu.  The device tree may list more (bi_ncpus),
	 * but nothing starts them yet.
	 */
	ncpus = 1;
	ncpus_fit = 1;
	ncpus_fit_mask = 0;

	init_param1();

	gd->mi.gd_prvspace = CPU_prvspace[0];
	gd->gd_mpidr = READ_SPECIALREG(mpidr_el1) & MPIDR_AFF_MASK;

	/*
	 * Physical memory.  Everything the firmware reports is usable
	 * except the kernel image (plus what pmap_bootstrap() carves
	 * off right behind it) and the device tree blob, which
	 * arm64_bootinfo keeps pointers into.
	 *
	 * XXX /memreserve/ entries and /reserved-memory nodes are not
	 * honoured yet.  QEMU has none; real firmware does.
	 */
	firstaddr = round_page(kernphys + (_end - kernbase));
	pmap_bootstrap(&firstaddr, kernphys);

	reserved[0].mr_start = kernphys;
	reserved[0].mr_size = firstaddr - kernphys;
	reserved[1].mr_start = fdt_pa;
	reserved[1].mr_size = bi->bi_fdt_size;
	n = 0;
	for (i = 0; i < bi->bi_nmem; ++i) {
		Realmem += bi->bi_mem[i].mr_size;
		dump_avail[i].phys_beg = bi->bi_mem[i].mr_start;
		dump_avail[i].phys_end = bi->bi_mem[i].mr_start +
		    bi->bi_mem[i].mr_size;
		add_phys_avail(bi->bi_mem[i].mr_start,
		    bi->bi_mem[i].mr_start + bi->bi_mem[i].mr_size,
		    reserved, 2, &n);
	}
	avail_start = phys_avail[0].phys_beg;
	avail_end = phys_avail[n - 1].phys_end;

	/*
	 * Now the machine-independent per-cpu and thread0 state.
	 */
	mi_gdinit(&gd->mi, 0);
	cpu_gdinit(gd, 0);
	mi_proc0init(&gd->mi, (struct user *)proc0paddr_buff);
	safepri = TDPRI_MAX;

	cninit();
	rand_initialize();
	init_param2(physmem);
	msgbufinit(msgbufp, MSGBUF_SIZE);

	thread0.td_pcb->pcb_flags = 0;
	thread0.td_pcb->pcb_ttbr0 = arm64_ttbr0_kernel;
	lwp0.lwp_md.md_regs = &proc0_tf;

	return ((u_long)thread0.td_pcb);
}

/*
 * First machine-dependent SYSINIT, run once the VM system and the
 * kernel memory allocator are up: announce the machine and carve out
 * the buffer cache's share of kernel virtual memory.  The second half
 * is the same on every platform and follows pc64's version.
 */
static void
cpu_startup(void *dummy __unused)
{
	struct arm64_bootinfo *bi = &arm64_bootinfo;
	caddr_t v;
	vm_size_t size = 0;
	vm_offset_t firstaddr;

	kprintf("%s", version);
	kprintf("Machine: %s\n", bi->bi_model ? bi->bi_model :
	    (bi->bi_compatible ? bi->bi_compatible : "unknown"));
	identify_cpu();
	kprintf("real memory  = %ju (%ju MB)\n", (intmax_t)Realmem,
	    (intmax_t)Realmem / 1024 / 1024);
	if (bootverbose) {
		int indx;

		kprintf("Physical memory chunk(s):\n");
		for (indx = 0; phys_avail[indx].phys_end != 0; ++indx) {
			vm_paddr_t size1;

			size1 = phys_avail[indx].phys_end -
			    phys_avail[indx].phys_beg;
			kprintf("0x%08jx - 0x%08jx, %ju bytes (%ju pages)\n",
			    (intmax_t)phys_avail[indx].phys_beg,
			    (intmax_t)phys_avail[indx].phys_end - 1,
			    (intmax_t)size1,
			    (intmax_t)(size1 / PAGE_SIZE));
		}
	}
	if (bootinfo_selftest())
		arm64_selftest();

	arm64_intr_init();

	/*
	 * Allocate space for system data structures.  The first
	 * available kernel virtual address is in "v".  As pages of
	 * kernel virtual memory are allocated, "v" is incremented.  As
	 * pages of memory are allocated and cleared, "firstaddr" is
	 * incremented.
	 */

	/*
	 * Make two passes.  The first pass calculates how much memory
	 * is needed and allocates it.  The second pass assigns virtual
	 * addresses to the various data structures.
	 */
	firstaddr = 0;
again:
	v = (caddr_t)firstaddr;

#define	valloc(name, type, num) \
	    (name) = (type *)v; v = (caddr_t)((name)+(num))

	/*
	 * The nominal buffer size (and minimum KVA allocation) is
	 * NBUFCALCSIZE.
	 */
	if (nbuf == 0) {
		long factor = NBUFCALCSIZE / 1024;		/* KB/nbuf */
		long kbytes = physmem * (PAGE_SIZE / 1024);	/* physmem */

		nbuf = 50;
		if (kbytes > 128 * 1024)
			nbuf += (kbytes - 128 * 1024) / (factor * 20);
		if (maxbcache && nbuf > maxbcache / NBUFCALCSIZE)
			nbuf = maxbcache / NBUFCALCSIZE;
		if ((size_t)nbuf * sizeof(struct buf) > MAXBUFSTRUCTSIZE) {
			kprintf("Warning: nbuf capped at %ld due to the "
				"reasonability limit\n", nbuf);
			nbuf = MAXBUFSTRUCTSIZE / sizeof(struct buf);
		}
	}

	/*
	 * Do not allow the buffer_map to be more then 1/2 the size of
	 * the kernel_map.
	 */
	if (nbuf > (virtual_end - virtual_start) / (MAXBSIZE * 2)) {
		nbuf = (virtual_end - virtual_start) / (MAXBSIZE * 2);
		kprintf("Warning: nbufs capped at %ld due to kvm\n", nbuf);
	}

	/*
	 * Do not allow the buffer_map to use more than 50% of available
	 * physical-equivalent memory.
	 */
	if (nbuf > physmem * PAGE_SIZE / (NBUFCALCSIZE * 2)) {
		nbuf = physmem * PAGE_SIZE / (NBUFCALCSIZE * 2);
		kprintf("Warning: nbufs capped at %ld due to physmem\n", nbuf);
	}

	/*
	 * Do not allow the sizeof(struct buf) * nbuf to exceed 1/4 of
	 * the valloc space which is just the virtual_end - virtual_start
	 * section.
	 */
	if (nbuf > (virtual_end - virtual_start) / (sizeof(struct buf) * 4)) {
		nbuf = (virtual_end - virtual_start) /
		       (sizeof(struct buf) * 4);
		kprintf("Warning: nbufs capped at %ld due to "
			"valloc considerations\n", nbuf);
	}

	nswbuf_mem = lmax(lmin(nbuf / 32, 512), 8);
	nswbuf_kva = lmax(lmin(nbuf / 4, 512), 16);

	valloc(swbuf_mem, struct buf, nswbuf_mem);
	valloc(swbuf_kva, struct buf, nswbuf_kva);
	valloc(buf, struct buf, nbuf);

	/*
	 * End of first pass, size has been calculated so allocate
	 * memory.
	 */
	if (firstaddr == 0) {
		size = (vm_size_t)(v - firstaddr);
		firstaddr = kmem_alloc(kernel_map, round_page(size),
				       VM_SUBSYS_BUF);
		if (firstaddr == 0)
			panic("startup: no room for tables");
		goto again;
	}

	/*
	 * End of second pass, addresses have been assigned.
	 */
	if ((vm_size_t)(v - firstaddr) != size)
		panic("startup: table size inconsistency");

	kmem_suballoc(kernel_map, clean_map, &clean_sva, &clean_eva,
		      ((vm_offset_t)(nbuf + 16) * MAXBSIZE) +
		      ((nswbuf_mem + nswbuf_kva) * MAXPHYS) + pager_map_size);
	kmem_suballoc(clean_map, buffer_map, &buffer_sva, &buffer_eva,
		      ((vm_offset_t)(nbuf + 16) * MAXBSIZE));
	buffer_map->system_map = 1;
	kmem_suballoc(clean_map, pager_map, &pager_sva, &pager_eva,
		      ((vm_offset_t)(nswbuf_mem + nswbuf_kva) * MAXPHYS) +
		      pager_map_size);
	pager_map->system_map = 1;
	kprintf("avail memory = %ju (%ju MB)\n",
		(uintmax_t)ptoa(vmstats.v_free_count + vmstats.v_dma_pages),
		(uintmax_t)ptoa(vmstats.v_free_count + vmstats.v_dma_pages) /
		1024 / 1024);
}

/*
 * Non-zero if the boot arguments ask for the primitive self-tests.
 */
int
bootinfo_selftest(void)
{
	return (kgetenv("selftest") != NULL ||
	    (arm64_bootinfo.bi_bootargs != NULL &&
	     strstr(arm64_bootinfo.bi_bootargs, "selftest") != NULL));
}

/*
 * Per-cpu data
 */

struct globaldata *
globaldata_find(int cpu)
{
	KKASSERT(cpu >= 0 && cpu < ncpus);
	return (&CPU_prvspace[cpu]->mdglobaldata.mi);
}

int
is_globaldata_space(vm_offset_t saddr, vm_offset_t eaddr)
{
	if (saddr >= (vm_offset_t)&CPU_prvspace_bsp &&
	    eaddr <= (vm_offset_t)(&CPU_prvspace_bsp + 1))
		return (TRUE);
	return (FALSE);
}

/*
 * Initialize machine-dependent portions of the global data structure:
 * the idle thread, which starts life in cpu_idle_restore().
 */
void
cpu_gdinit(struct mdglobaldata *gd, int cpu)
{
	if (cpu)
		gd->mi.gd_curthread = &gd->mi.gd_idlethread;

	lwkt_init_thread(&gd->mi.gd_idlethread,
			gd->mi.gd_prvspace->idlestack,
			sizeof(gd->mi.gd_prvspace->idlestack),
			0, &gd->mi);
	lwkt_set_comm(&gd->mi.gd_idlethread, "idle_%d", cpu);
	gd->mi.gd_idlethread.td_switch = cpu_lwkt_switch;
	gd->mi.gd_idlethread.td_sp -= 2 * sizeof(void *);
	*(void **)gd->mi.gd_idlethread.td_sp = cpu_idle_restore;
}

/*
 * The idle loop.  Entered by each cpu's idle thread with a critical
 * section held and never returns.
 */
void
cpu_idle(void)
{
	struct thread *td = curthread;

	crit_exit();
	KKASSERT(td->td_critcount == 0);
	for (;;) {
		/* See if there are any LWKTs ready to go. */
		lwkt_switch();

		/*
		 * Nothing to run.  Sleep until an interrupt, unless
		 * something became pending in the meantime.  WFI wakes
		 * on a pending interrupt even while interrupts are
		 * masked, so checking and then sleeping with them
		 * masked cannot miss one.
		 */
		cpu_disable_intr();
		if ((td->td_gd->gd_reqflags & RQF_IDLECHECK_WK_MASK) == 0) {
			if (cpu_idle_hook)
				cpu_idle_hook();
			else
				halt();
		}
		cpu_enable_intr();
		splz();
	}
}

void
cpu_inhibit_deep_sleep(int set __unused)
{
}

/*
 * Halt, reset and power off
 */

void
cpu_poweroff(void)
{
	cpu_disable_intr();
	if (arm64_bootinfo.bi_psci != 0)
		psci_call(PSCI_SYSTEM_OFF, 0, 0, arm64_bootinfo.bi_psci == 2);
	for (;;)
		halt();
}

void
cpu_reset(void)
{
	cpu_disable_intr();
	if (arm64_bootinfo.bi_psci != 0)
		psci_call(PSCI_SYSTEM_RESET, 0, 0,
		    arm64_bootinfo.bi_psci == 2);
	for (;;)
		halt();
}

void
cpu_halt(void)
{
	cpu_disable_intr();
	for (;;)
		halt();
}

static void
arm64_shutdown_final(void *arg __unused, int howto)
{
	if (howto & RB_POWEROFF)
		cpu_poweroff();
}

static void
arm64_shutdown_init(void *arg __unused)
{
	EVENTHANDLER_REGISTER(shutdown_final, arm64_shutdown_final, NULL,
	    SHUTDOWN_PRI_LAST);
}
SYSINIT(arm64shutdown, SI_BOOT2_PROC0, SI_ORDER_ANY, arm64_shutdown_init,
    NULL);
