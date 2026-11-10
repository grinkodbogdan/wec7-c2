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
 * Structure offsets and constants needed by the assembly sources.
 * kern/genassym.sh turns the symbols this file defines into assym.s.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/assym.h>
#include <sys/thread.h>
#include <sys/globaldata.h>
#include <sys/proc.h>
#include <sys/errno.h>

#include <cpu/frame.h>
#include <machine/globaldata.h>
#include <machine/pcb.h>
#include <machine/vmparam.h>

ASSYM(GD_CURTHREAD, __offsetof(struct mdglobaldata, mi.gd_curthread));
ASSYM(GD_CPUID, __offsetof(struct mdglobaldata, mi.gd_cpuid));
ASSYM(GD_INTR_NESTING_LEVEL,
    __offsetof(struct mdglobaldata, mi.gd_intr_nesting_level));

ASSYM(TD_PCB, __offsetof(struct thread, td_pcb));
ASSYM(TD_SP, __offsetof(struct thread, td_sp));
ASSYM(TD_LWP, __offsetof(struct thread, td_lwp));
ASSYM(TD_FLAGS, __offsetof(struct thread, td_flags));
ASSYM(TD_CRITCOUNT, __offsetof(struct thread, td_critcount));
ASSYM(TD_PREEMPTED, __offsetof(struct thread, td_preempted));

ASSYM(PCB_X19, __offsetof(struct pcb, pcb_x[0]));
ASSYM(PCB_X20, __offsetof(struct pcb, pcb_x[1]));
ASSYM(PCB_TTBR0, __offsetof(struct pcb, pcb_ttbr0));
ASSYM(PCB_TPIDR_EL0, __offsetof(struct pcb, pcb_tpidr_el0));
ASSYM(PCB_ONFAULT, __offsetof(struct pcb, pcb_onfault));
ASSYM(PCB_ONFAULT_SP, __offsetof(struct pcb, pcb_onfault_sp));

ASSYM(TF_SP, __offsetof(struct trapframe, tf_sp));
ASSYM(TF_LR, __offsetof(struct trapframe, tf_lr));
ASSYM(TF_ELR, __offsetof(struct trapframe, tf_elr));
ASSYM(TF_SPSR, __offsetof(struct trapframe, tf_spsr));
ASSYM(TF_ESR, __offsetof(struct trapframe, tf_esr));
ASSYM(TF_FAR, __offsetof(struct trapframe, tf_far));
ASSYM(TF_X, __offsetof(struct trapframe, tf_x));
ASSYM(TF_SIZE, sizeof(struct trapframe));

ASSYM(EFAULT, EFAULT);
ASSYM(ENAMETOOLONG, ENAMETOOLONG);
ASSYM(VM_MAX_USER_ADDRESS, VM_MAX_USER_ADDRESS);
