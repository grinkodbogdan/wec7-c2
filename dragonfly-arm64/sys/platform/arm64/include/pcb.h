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

#ifndef _MACHINE_PCB_H_
#define _MACHINE_PCB_H_

/*
 * arm64 process control block
 */
#include <cpu/fpu.h>

struct pcb {
	/*
	 * Callee-saved state switched by cpu_heavy_switch()/cpu_lwkt_switch():
	 * x19-x28, x29 (fp), x30 (lr), sp.
	 */
	register_t	pcb_x[10];	/* x19 - x28 */
	register_t	pcb_fp;
	register_t	pcb_lr;
	register_t	pcb_sp;

	register_t	pcb_ttbr0;	/* user page table + ASID */
	register_t	pcb_tpidr_el0;	/* user thread pointer */
	register_t	pcb_tpidrro_el0;

	u_long		pcb_flags;
	caddr_t		pcb_onfault;	/* copyin/out fault recovery */
	register_t	pcb_onfault_sp; /* validate onfault */

	struct savefpu	pcb_save;
};

#define	PCB_DBREGS	0x00000002	/* process using debug registers */
#define	PCB_FPUINITDONE	0x00000008	/* fpu state is initialized */

#ifdef _KERNEL
void	savectx(struct pcb *);
#endif

#endif /* _MACHINE_PCB_H_ */
