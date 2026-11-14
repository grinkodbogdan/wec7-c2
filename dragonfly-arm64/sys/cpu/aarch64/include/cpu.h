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

#ifndef _CPU_CPU_H_
#define	_CPU_CPU_H_

/*
 * Definitions unique to aarch64 cpu support.
 */
#ifndef _CPU_ARMREG_H_
#include <cpu/armreg.h>
#endif
#ifndef _CPU_FRAME_H_
#include <machine/frame.h>
#endif

#define	cpu_exec(p)	/* nothing */
#define cpu_swapin(p)	/* nothing */
#define cpu_setstack(lp, ap)		((lp)->lwp_md.md_regs->tf_sp = (ap))

/*
 * Preempting the current process if in interrupt from user mode is
 * arranged by setting an AST request flag in the per-cpu globaldata.
 * The flags are checked on the way back to user mode and, for the LWKT
 * ones, on return to a non-critical-section kernel context.
 *
 * These are identical to x86_64; nothing about them is cpu specific
 * other than living in this header.
 */
#define	need_lwkt_resched()	\
    atomic_set_int(&mycpu->gd_reqflags, RQF_AST_LWKT_RESCHED)
#define	need_user_resched()	\
    atomic_set_int(&mycpu->gd_reqflags, RQF_AST_USER_RESCHED)
#define	need_proftick()		\
    atomic_set_int(&mycpu->gd_reqflags, RQF_AST_OWEUPC)
#define	need_ipiq()		\
    atomic_set_int(&mycpu->gd_reqflags, RQF_IPIQ)
#define	need_timer()		\
    atomic_set_int(&mycpu->gd_reqflags, RQF_TIMER)
#define	signotify()		\
    atomic_set_int(&mycpu->gd_reqflags, RQF_AST_SIGNAL)
#define	clear_user_resched()	\
    atomic_clear_int(&mycpu->gd_reqflags, RQF_AST_USER_RESCHED)
#define	clear_lwkt_resched()	\
    atomic_clear_int(&mycpu->gd_reqflags, RQF_AST_LWKT_RESCHED)
#define	clear_quickret()	\
    atomic_clear_int(&mycpu->gd_reqflags, RQF_QUICKRET)
#define	user_resched_wanted()	\
    (mycpu->gd_reqflags & RQF_AST_USER_RESCHED)
#define	lwkt_resched_wanted()	\
    (mycpu->gd_reqflags & RQF_AST_LWKT_RESCHED)
#define	any_resched_wanted()	\
    (mycpu->gd_reqflags & (RQF_AST_LWKT_RESCHED|RQF_AST_USER_RESCHED))
#define	sched_action_wanted_gd(gd) \
    ((gd)->gd_reqflags & RQF_SCHED_MASK)

/*
 * CTL_MACHDEP definitions.
 */
#define	CPU_CONSDEV		1	/* cdev_t: console terminal device */
#define	CPU_ADJKERNTZ		2	/* int: timezone offset (seconds) */
#define	CPU_DISRTCSET		3	/* int: disable resettodr() call */
#define	CPU_BOOTINFO		4	/* struct: bootinfo */
#define	CPU_WALLCLOCK		5	/* int: indicates wall CMOS clock */
#define	CPU_MAXID		6	/* number of valid machdep ids */

#ifdef _KERNEL

struct lwp;

extern char	btext[];
extern char	etext[];

void	fork_trampoline (void);
void	generic_lwp_return (struct lwp *, struct trapframe *);
void	fork_return (struct lwp *, struct trapframe *);

#endif

#endif /* !_CPU_CPU_H_ */
