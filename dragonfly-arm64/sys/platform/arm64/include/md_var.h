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

#ifndef _MACHINE_MD_VAR_H_
#define	_MACHINE_MD_VAR_H_

#ifndef _SYS_TYPES_H_
#include <sys/types.h>
#endif
#include <machine/globaldata.h>

/*
 * Miscellaneous machine-dependent declarations for the arm64 platform.
 */

struct trapframe;

#define	ARM64_MAXMEMREGIONS	16

struct arm64_memregion {
	vm_paddr_t	mr_start;
	vm_size_t	mr_size;
};

/*
 * Everything early boot learns from the flattened device tree.
 */
struct arm64_bootinfo {
	const char	*bi_model;		/* root "model" */
	const char	*bi_compatible;		/* root "compatible" (first) */
	const char	*bi_bootargs;		/* /chosen bootargs */
	const char	*bi_stdout;		/* /chosen stdout-path */
	struct arm64_memregion bi_mem[ARM64_MAXMEMREGIONS];
	int		bi_nmem;
	int		bi_ncpus;
	vm_paddr_t	bi_uart_base;		/* arm,pl011 */
	vm_paddr_t	bi_gicd_base;		/* GICv2 distributor */
	vm_paddr_t	bi_gicc_base;		/* GICv2 cpu interface */
	int		bi_gic_version;		/* 0 = none found */
	int		bi_timer_irq;		/* virtual timer INTID */
	int		bi_psci;		/* 0 none, 1 hvc, 2 smc */
	u_int		bi_fdt_size;
	vm_paddr_t	bi_initrd_start;	/* /chosen linux,initrd-* */
	vm_paddr_t	bi_initrd_end;
};

extern struct arm64_bootinfo arm64_bootinfo;
extern uint64_t	arm64_ttbr0_kernel;

extern	char	sigcode[];
extern	int	szsigcode;
extern	uint64_t *vm_page_dump;
extern	vm_offset_t vm_page_dump_size;
extern	int	busdma_swi_pending;
extern	void	(*cpu_idle_hook)(void);

struct dumperinfo;
struct intrframe;
struct mdglobaldata;
struct thread;

u_long	initarm(vm_paddr_t fdt_pa, vm_paddr_t kernphys);
int	bootinfo_selftest(void);
void	arm64_selftest(void);
void	trap_el0(struct trapframe *frame);
void	arm64_user_return(struct trapframe *frame);

int	fdt_scan(const void *fdt, struct arm64_bootinfo *bi);

void	cninit_early(vm_paddr_t uart_base);
void	cnputc_early(int c);
void	cnputs_early(const char *s);

void	identify_cpu(void);

void	trap(struct trapframe *frame);
void	trap_fatal(struct trapframe *frame, int from_el1) __dead2;

void	arm64_intr_init(void);
void	arm64_intr(struct trapframe *frame);
int	gic_init(vm_paddr_t gicd, vm_paddr_t gicc);
void	gic_enable_irq(int irq);
void	gic_disable_irq(int irq);
int	gic_ack(void);
void	gic_eoi(int iar);

void	clock_intr(struct intrframe *frame);
void	clock_intr_quiesce(void);

int64_t	psci_call(uint64_t fn, uint64_t a1, uint64_t a2, int use_smc);
void	cpu_poweroff(void) __dead2;

void	busdma_swi(void);
void	cpu_gdinit(struct mdglobaldata *gd, int cpu);
void	cpu_idle(void);
void	cpu_idle_restore(void);		/* cannot be called from C */
void	cpu_heavy_restore(void);	/* cannot be called from C */
void	cpu_kthread_restore(void);	/* cannot be called from C */
void	cpu_reset(void);
void	dump_add_page(vm_paddr_t);
void	dump_drop_page(vm_paddr_t);
void	minidumpsys(struct dumperinfo *);
void	pagezero(void *addr);

#endif /* !_MACHINE_MD_VAR_H_ */
