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

#ifndef _MACHINE_SMP_H_
#define _MACHINE_SMP_H_

#ifdef _KERNEL

#ifndef LOCORE

#include <machine/pcb.h>

/*
 * Secondary cpus are started through PSCI CPU_ON and identified by
 * their MPIDR_EL1 affinity value.  IPIs are GIC software generated
 * interrupts (SGIs).
 */
void	initializecpu(int cpu);

extern volatile cpumask_t	stopped_cpus;
extern volatile cpumask_t	started_cpus;

extern void (*cpustop_restartfunc) (void);

extern struct pcb		stoppcbs[];

void	mp_start		(void);
void	mp_announce		(void);
void	init_secondary		(int cpu);
int	stop_cpus		(cpumask_t);
void	ap_init			(void);
int	restart_cpus		(cpumask_t);
void	cpu_send_ipiq		(int);
int	cpu_send_ipiq_passive	(int);

extern cpumask_t		smp_active_mask;

/*
 * kern/subr_cpu_topology.c walks cpus by "APIC id".  aarch64 has no
 * such thing; hardware identifies cpus by MPIDR affinity, which the
 * platform translates to a dense logical cpu id at boot.  The topology
 * code only needs an enumerable id space, so the logical id serves.
 */
extern int ncpus;
extern int naps;

#define	CPUID_TO_APICID(cpuid)		(cpuid)
#define	APICID_TO_CPUID(apicid)		(apicid)

static __inline int
get_apicid_from_cpuid(int cpuid)
{
	return (cpuid);
}

static __inline int
get_cpuid_from_apicid(int apicid)
{
	/*
	 * naps, not ncpus: the topology is built (SI_BOOT2_CPU_TOPOLOGY)
	 * after the cpus are counted but before they are started.
	 */
	return (apicid >= 0 && apicid <= naps ? apicid : -1);
}

void	detect_cpu_topology(void);
int	get_chip_ID(int cpuid);
int	get_chip_ID_from_APICID(int apicid);
int	get_core_number_within_chip(int cpuid);
int	get_logical_CPU_number_within_core(int cpuid);

#endif /* !LOCORE */
#endif /* _KERNEL */
#endif /* _MACHINE_SMP_H_ */
