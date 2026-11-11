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
#include <vfs/hammer2/zlib/hammer2_zlib.h>

/* In the kernel zlib, not declared by its header. */
int	inflateInit2_(z_streamp strm, int windowBits, const char *version,
	    int stream_size);
#include <sys/linker.h>
#include <sys/malloc.h>
#include <sys/libkern.h>

#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/vm_kern.h>
#include <vm/vm_object.h>
#include <vm/vm_page.h>
#include <vm/vm_map.h>
#include <vm/vm_pager.h>
#include <vm/vm_extern.h>

#include <sys/thread2.h>
#include <sys/mplock2.h>

#include <machine/cpu.h>
#include <machine/cpufunc.h>
#include <machine/globaldata.h>
#include <machine/md_var.h>
#include <machine/pcb.h>
#include <machine/pmap.h>
#include <machine/vmparam.h>

#define	PSCI_SYSTEM_OFF		0x84000008UL
#define	PSCI_SYSTEM_RESET	0x84000009UL

#define	PHYS_AVAIL_ENTRIES	(2 * (ARM64_MAXMEMREGIONS + ARM64_MAXRESERVED) + 8)

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

/* As on pc64: memory sizes userland reads through sysctl. */
static int
sysctl_hw_physmem(SYSCTL_HANDLER_ARGS)
{
	u_long pmem = ctob(physmem);

	return (sysctl_handle_long(oidp, &pmem, 0, req));
}
SYSCTL_PROC(_hw, HW_PHYSMEM, physmem, CTLTYPE_ULONG|CTLFLAG_RD,
	0, 0, sysctl_hw_physmem, "LU",
	"Total system memory in bytes (number of pages * page size)");

static int
sysctl_hw_usermem(SYSCTL_HANDLER_ARGS)
{
	u_long usermem = ctob(physmem - vmstats.v_wire_count);

	return (sysctl_handle_long(oidp, &usermem, 0, req));
}
SYSCTL_PROC(_hw, HW_USERMEM, usermem, CTLTYPE_ULONG|CTLFLAG_RD,
	0, 0, sysctl_hw_usermem, "LU", "");
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

/*
 * Module metadata in the format loader(8) produces, describing the
 * initial ramdisk as a preloaded memory disk image for md(4).
 */
static uint32_t	preload_buf[32];

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

static uint32_t *
preload_add(uint32_t *p, uint32_t type, const void *data, uint32_t size)
{
	*p++ = type;
	*p++ = size;
	bcopy(data, p, size);
	return (p + howmany(size, sizeof(u_long)) * (sizeof(u_long) / 4));
}

/*
 * A ramdisk passed the Linux way (QEMU -initrd, a boot image's ramdisk)
 * becomes md0.  There is no loader(8) to preload modules, so fabricate
 * the one metadata record md(4) looks for.
 */
static void preload_initrd_va(vm_offset_t addr, size_t size);
static vm_paddr_t initrd_pa_start, initrd_pa_end;

static void
preload_initrd(vm_paddr_t start, vm_paddr_t end)
{
	initrd_pa_start = start;
	initrd_pa_end = end;
	preload_initrd_va(PHYS_TO_DMAP(start), end - start);
}

static void
preload_initrd_va(vm_offset_t addr, size_t size)
{
	static const char name[] = "initrd";
	static const char type[] = "md_image";
	uint32_t *p = preload_buf;

	p = preload_add(p, MODINFO_NAME, name, sizeof(name));
	p = preload_add(p, MODINFO_TYPE, type, sizeof(type));
	p = preload_add(p, MODINFO_ADDR, &addr, sizeof(addr));
	p = preload_add(p, MODINFO_SIZE, &size, sizeof(size));
	*p++ = MODINFO_END;
	*p++ = 0;
	preload_metadata = (caddr_t)preload_buf;
}

/*
 * A gzip-compressed ramdisk (the boot image's ramdisk can be
 * root.iso.gz): inflate it into kernel memory just before md(4) attaches
 * the preloaded image, and point the preload record at the result.  The
 * bootloader's region limits the compressed size, not the uncompressed.
 * Uses the zlib HAMMER2 brings into the kernel; it has no gzip wrapper
 * support, so the gzip header is parsed here and the data inflated raw.
 */
static void
initrd_gunzip(void *dummy __unused)
{
	const uint8_t *gz;
	size_t len, hdr, out_size;
	z_stream z;
	uint8_t flg;
	void *out;
	int error;

	if (initrd_pa_end <= initrd_pa_start)
		return;
	gz = (const uint8_t *)PHYS_TO_DMAP(initrd_pa_start);
	len = initrd_pa_end - initrd_pa_start;
	if (len < 18 || gz[0] != 0x1f || gz[1] != 0x8b || gz[2] != 8)
		return;			/* not gzip: use it as it is */

	flg = gz[3];
	hdr = 10;
	if (flg & 0x04)			/* FEXTRA */
		hdr += 2 + (gz[hdr] | gz[hdr + 1] << 8);
	if (flg & 0x08)			/* FNAME */
		while (hdr < len && gz[hdr++] != 0)
			;
	if (flg & 0x10)			/* FCOMMENT */
		while (hdr < len && gz[hdr++] != 0)
			;
	if (flg & 0x02)			/* FHCRC */
		hdr += 2;
	out_size = gz[len - 4] | gz[len - 3] << 8 | gz[len - 2] << 16 |
	    (size_t)gz[len - 1] << 24;	/* ISIZE: size mod 4GB */

	out = (void *)kmem_alloc(kernel_map, round_page(out_size),
	    VM_SUBSYS_UNKNOWN);
	if (out == NULL) {
		kprintf("initrd: no memory to inflate %zu bytes\n", out_size);
		return;
	}
	bzero(&z, sizeof(z));
	z.next_in = __DECONST(uint8_t *, gz + hdr);
	z.avail_in = len - hdr - 8;
	z.next_out = out;
	z.avail_out = out_size;
	error = inflateInit2(&z, -MAX_WBITS);
	if (error == Z_OK)
		error = inflate(&z, Z_FINISH);
	inflateEnd(&z);
	if (error != Z_STREAM_END || z.total_out != out_size) {
		kprintf("initrd: inflate failed (%d, %lu of %zu bytes)\n",
		    error, (u_long)z.total_out, out_size);
		kmem_free(kernel_map, (vm_offset_t)out, round_page(out_size));
		return;
	}
	kprintf("initrd: inflated %zu bytes to %zu\n", len, out_size);
	preload_initrd_va((vm_offset_t)out, out_size);
}
SYSINIT(initrd_gunzip, SI_SUB_DRIVERS, SI_ORDER_FIRST, initrd_gunzip, NULL);

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
 * A progress line for a verbose boot, usable before the console layer
 * exists: it goes straight to the serial port, the screen and the RAM
 * log.  On new hardware these are the only way to tell where an early
 * hang happened.
 */
static int	cons_ready;	/* kprintf() has a console */

void
boot_mark(const char *s)
{
	if (!bootverbose)
		return;
	if (cons_ready)
		kprintf("%s", s);	/* shares kprintf's message lock */
	else
		cnputs_early(s);
}

/*
 * With a verbose boot, say what the firmware handed over before the
 * machine-independent kernel starts.  On new hardware this is the
 * first thing worth photographing: everything that can go wrong in
 * early memory setup follows from these numbers.
 */
static void
boot_report(vm_paddr_t fdt_pa, vm_paddr_t kernphys, vm_paddr_t firstaddr,
	    const struct arm64_memregion *reserved, int nreserved)
{
	struct arm64_bootinfo *bi = &arm64_bootinfo;
	int i;

	kprintf("boot: EL%d midr %08jx cntfrq %ju dt-freq %u\n",
	    (int)CURRENTEL_EL(READ_SPECIALREG(CurrentEL)),
	    (uintmax_t)READ_SPECIALREG(midr_el1),
	    (uintmax_t)READ_SPECIALREG(cntfrq_el0), bi->bi_timer_freq);
	kprintf("boot: kernel %jx-%jx fdt %jx+%x initrd %jx-%jx\n",
	    (uintmax_t)kernphys, (uintmax_t)firstaddr, (uintmax_t)fdt_pa,
	    bi->bi_fdt_size, (uintmax_t)bi->bi_initrd_start,
	    (uintmax_t)bi->bi_initrd_end);
	kprintf("boot: fb %jx %ux%u stride %u bpp %u wdt %jx\n",
	    (uintmax_t)bi->bi_fb_base, bi->bi_fb_width, bi->bi_fb_height,
	    bi->bi_fb_stride, bi->bi_fb_bpp, (uintmax_t)bi->bi_wdt_base);
	kprintf("boot: gic v%d %jx %jx timer irq %d psci %d cpus %d\n",
	    bi->bi_gic_version, (uintmax_t)bi->bi_gicd_base,
	    (uintmax_t)bi->bi_gicc_base, bi->bi_timer_irq, bi->bi_psci,
	    bi->bi_ncpus);
	for (i = 0; i < bi->bi_nmem; ++i) {
		kprintf("boot: ram %jx-%jx\n",
		    (uintmax_t)bi->bi_mem[i].mr_start,
		    (uintmax_t)(bi->bi_mem[i].mr_start +
		    bi->bi_mem[i].mr_size));
	}
	for (i = 0; i < nreserved; ++i) {
		kprintf("boot: resv %jx-%jx%s\n",
		    (uintmax_t)reserved[i].mr_start,
		    (uintmax_t)(reserved[i].mr_start + reserved[i].mr_size),
		    i < 2 ? (i == 0 ? " kernel" : " fdt") : "");
	}
	kprintf("boot: %lu MB unmapped as firmware memory\n",
	    arm64_nomap_pages >> (20 - PAGE_SHIFT));
	for (i = 0; phys_avail[i].phys_end != 0; ++i) {
		kprintf("boot: free %jx-%jx\n",
		    (uintmax_t)phys_avail[i].phys_beg,
		    (uintmax_t)phys_avail[i].phys_end);
	}
	kprintf("boot: entering mi_startup\n");
}

/*
 * Progress markers for a verbose boot, one after each early stage of
 * mi_startup().  If the machine stops, the last one on the screen says
 * which stage it was in.
 */
static void
boot_stage(void *arg)
{
	if (bootverbose)
		kprintf("boot: %s done\n", (const char *)arg);
}
SYSINIT(stage_lock, SI_BOOT1_LOCK, SI_ORDER_ANY, boot_stage, "locks");
SYSINIT(stage_vm, SI_BOOT1_VM, SI_ORDER_ANY, boot_stage, "vm");
SYSINIT(stage_alloc, SI_BOOT1_ALLOCATOR, SI_ORDER_ANY, boot_stage,
    "allocator");
SYSINIT(stage_kmalloc, SI_BOOT1_KMALLOC, SI_ORDER_ANY, boot_stage, "kmalloc");
SYSINIT(stage_post, SI_BOOT1_POST, SI_ORDER_ANY, boot_stage, "boot1");
SYSINIT(stage_crit, SI_BOOT2_LEAVE_CRIT, SI_ORDER_ANY, boot_stage,
    "interrupts on");
SYSINIT(stage_presmp, SI_BOOT2_PRESMP, SI_ORDER_ANY, boot_stage, "presmp");

/*
 * Sort a table of physical ranges by start address, in place.  The
 * machine-independent VM (vm_page_startup()) takes phys_avail[0] as
 * the lowest memory and the last entry as the highest, but firmware
 * lists memory banks in whatever order it likes: the Xiaomi Mi 5's
 * bootloader puts the bank above 4GB first.
 */
static void
sort_phys(vm_phystable_t *tab, int n)
{
	vm_phystable_t t;
	int i, j;

	for (i = 1; i < n; ++i) {
		t = tab[i];
		for (j = i; j > 0 && tab[j - 1].phys_beg > t.phys_beg; --j)
			tab[j] = tab[j - 1];
		tab[j] = t;
	}
}

/*
 * Boot argument "memrev": present memory as two banks split at 4GB,
 * upper bank first, the way the Mi 5's bootloader does.  QEMU always
 * describes its RAM as one bank, so this is how that layout gets
 * tested in the emulator.
 */
static void
simulate_bank_order(struct arm64_bootinfo *bi)
{
	struct arm64_memregion *m = &bi->bi_mem[0];
	vm_paddr_t split = 0x100000000UL;

	if (bi->bi_bootargs == NULL || strstr(bi->bi_bootargs, "memrev") == NULL)
		return;
	if (bi->bi_nmem != 1 || m->mr_start >= split ||
	    m->mr_start + m->mr_size <= split)
		return;
	bi->bi_mem[1].mr_start = m->mr_start;
	bi->bi_mem[1].mr_size = split - m->mr_start;
	m->mr_size = m->mr_start + m->mr_size - split;
	m->mr_start = split;
	bi->bi_nmem = 2;
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
	static struct arm64_memregion reserved[ARM64_MAXRESERVED + 4];
	int nreserved = 2;
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
	cninit_early(bi->bi_uart_base);
	fbcons_bootarg(bi);
	if (msm8996_early_init(bi))
		fbcons_flush_hook = msm8996_fb_flush;
	fbcons_init(bi);
	if (!fdt_ok || bi->bi_nmem == 0) {
		cnputs_early("\nDragonFly/arm64: no usable device tree, "
		    "cannot determine memory layout\n");
		for (;;)
			halt();
	}
	parse_bootargs(bi->bi_bootargs);
	boot_mark("\nboot: DragonFly/arm64 initarm, device tree read\n");
	if (bi->bi_wdt_base != 0)
		boot_mark("boot: msm8996: watchdog stopped\n");

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
	 * off right behind it), the device tree blob, which
	 * arm64_bootinfo keeps pointers into, the initial ramdisk, a
	 * firmware framebuffer, and whatever the device tree marks as
	 * reserved (on a phone: memory owned by the modem, the secure
	 * world and other firmware).
	 */
	simulate_bank_order(bi);

	firstaddr = round_page(kernphys + (_end - kernbase));
	boot_mark("boot: page tables\n");
	pmap_bootstrap(&firstaddr, kernphys);
	boot_mark("boot: page tables done\n");

	reserved[0].mr_start = kernphys;
	reserved[0].mr_size = firstaddr - kernphys;
	reserved[1].mr_start = fdt_pa;
	reserved[1].mr_size = bi->bi_fdt_size;
	if (bi->bi_initrd_end > bi->bi_initrd_start) {
		reserved[2].mr_start = bi->bi_initrd_start;
		reserved[2].mr_size = bi->bi_initrd_end - bi->bi_initrd_start;
		++nreserved;
		preload_initrd(bi->bi_initrd_start, bi->bi_initrd_end);
	}
	if (bi->bi_fb_base != 0) {
		reserved[nreserved].mr_start = bi->bi_fb_base;
		reserved[nreserved].mr_size =
		    (vm_size_t)bi->bi_fb_stride * bi->bi_fb_height;
		++nreserved;
	}
	for (i = 0; i < bi->bi_nresv; ++i)
		reserved[nreserved++] = bi->bi_resv[i];
	n = 0;
	for (i = 0; i < bi->bi_nmem; ++i) {
		Realmem += bi->bi_mem[i].mr_size;
		dump_avail[i].phys_beg = bi->bi_mem[i].mr_start;
		dump_avail[i].phys_end = bi->bi_mem[i].mr_start +
		    bi->bi_mem[i].mr_size;
		add_phys_avail(bi->bi_mem[i].mr_start,
		    bi->bi_mem[i].mr_start + bi->bi_mem[i].mr_size,
		    reserved, nreserved, &n);
	}
	sort_phys(phys_avail, n);
	sort_phys(dump_avail, bi->bi_nmem);
	avail_start = phys_avail[0].phys_beg;
	avail_end = phys_avail[n - 1].phys_end;

	/*
	 * Now the machine-independent per-cpu and thread0 state.
	 */
	boot_mark("boot: per-cpu data, thread0\n");
	mi_gdinit(&gd->mi, 0);
	cpu_gdinit(gd, 0);
	mi_proc0init(&gd->mi, (struct user *)proc0paddr_buff);
	safepri = TDPRI_MAX;

	/*
	 * The boot thread holds the MP lock until the other cpus have
	 * joined (ap_finish(), mp_machdep.c).
	 */
	cpu_get_initial_mplock();

	cninit();
	cons_ready = 1;
	rand_initialize();
	init_param2(physmem);
	msgbufinit(msgbufp, MSGBUF_SIZE);

	/*
	 * The interrupt controller has to be in a known state before
	 * mi_startup() unmasks interrupts (leavecrit, SI_BOOT2_LEAVE_CRIT):
	 * a bootloader may leave it enabled with interrupts pending, and
	 * one that cannot be acknowledged is taken again and again
	 * forever.  This is what stopped the first boot on a Xiaomi Mi 5
	 * right after the copyright notice.
	 */
	arm64_simulate_bootloader_irq();
	boot_mark("boot: interrupt controller\n");
	arm64_intr_init();

	thread0.td_pcb->pcb_flags = 0;
	thread0.td_pcb->pcb_ttbr0 = arm64_ttbr0_kernel;
	lwp0.lwp_md.md_regs = &proc0_tf;

	if (bootverbose)
		boot_report(fdt_pa, kernphys, firstaddr, reserved, nreserved);

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
	if (bi->bi_fb_moved_from != 0) {
		kprintf("fb: moved off the ramdisk, from 0x%jx\n",
		    (uintmax_t)bi->bi_fb_moved_from);
	}
	if (bi->bi_fb_base != 0) {
		kprintf("fb0: %ux%u, %u bpp framebuffer at 0x%jx\n",
		    bi->bi_fb_width, bi->bi_fb_height, bi->bi_fb_bpp,
		    (uintmax_t)bi->bi_fb_base);
	}
	if (bi->bi_nresv != 0) {
		kprintf("%d reserved memory region(s) from the device tree, "
		    "%lu MB of them unmapped\n", bi->bi_nresv,
		    arm64_nomap_pages >> (20 - PAGE_SHIFT));
	}
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
	int i;

	for (i = 0; i < ncpus; ++i) {
		if (CPU_prvspace[i] != NULL &&
		    saddr >= (vm_offset_t)CPU_prvspace[i] &&
		    eaddr <= (vm_offset_t)(CPU_prvspace[i] + 1))
			return (TRUE);
	}
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
