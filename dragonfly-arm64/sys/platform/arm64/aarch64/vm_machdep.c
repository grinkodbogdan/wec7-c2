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
 * Machine-dependent thread and process setup.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/proc.h>
#include <sys/interrupt.h>
#include <sys/unistd.h>
#include <sys/lwp.h>

#include <vm/vm.h>
#include <vm/pmap.h>
#include <vm/vm_map.h>
#include <vm/vm_extern.h>

#include <sys/thread2.h>

#include <machine/cpu.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/pcb.h>
#include <machine/pmap.h>
#include <machine/vmparam.h>

/* Indexes into the switch frame built by swtch.S, in 8 byte slots. */
#define	SWF_RESTORE	0
#define	SWF_DAIF	1
#define	SWF_X19		2
#define	SWF_X20		3
#define	SWF_X29		12
#define	SWF_X30		13
#define	SWF_SLOTS	14

thread_t cpu_exit_switch(thread_t ntd);

/*
 * Finish a fork operation, with lwp lp2 nearly set up.  Copy and
 * update the pcb and set up the kernel stack so that the first switch
 * to the child lands in fork_trampoline, which calls fork_return().
 */
void
cpu_fork(struct lwp *lp1, struct lwp *lp2, int flags)
{
	struct pcb *pcb2;
	uint64_t *swf;

	if ((flags & RFPROC) == 0)
		return;

	/* The live FP/SIMD registers are part of what the child inherits. */
	if (lp1->lwp_thread == curthread)
		fpu_save(&lp1->lwp_thread->td_pcb->pcb_save);

	/* Copy lp1's pcb, the child's trapframe sits right below it. */
	pcb2 = lp2->lwp_thread->td_pcb;
	*pcb2 = *lp1->lwp_thread->td_pcb;
	lp2->lwp_md.md_regs = (struct trapframe *)pcb2 - 1;
	bcopy(lp1->lwp_md.md_regs, lp2->lwp_md.md_regs,
	      sizeof(*lp2->lwp_md.md_regs));

	pcb2->pcb_ttbr0 = vmspace_pmap(lp2->lwp_proc->p_vmspace)->pm_ttbr0;
	pcb2->pcb_onfault = NULL;

	/*
	 * A switch frame as cpu_heavy_switch would have left it, so the
	 * ordinary restore path "returns" into fork_trampoline with
	 * x19 = function and x20 = argument, interrupts enabled.
	 */
	swf = (uint64_t *)lp2->lwp_md.md_regs - SWF_SLOTS;
	bzero(swf, SWF_SLOTS * sizeof(*swf));
	swf[SWF_RESTORE] = (uint64_t)cpu_heavy_restore;
	swf[SWF_DAIF] = 0;
	swf[SWF_X19] = (uint64_t)fork_return;
	swf[SWF_X20] = (uint64_t)lp2;
	swf[SWF_X30] = (uint64_t)fork_trampoline;
	lp2->lwp_thread->td_sp = (char *)swf;

	bcopy(&lp1->lwp_thread->td_tls, &lp2->lwp_thread->td_tls,
	      sizeof(lp2->lwp_thread->td_tls));
}

/*
 * Intercept the return address from a freshly forked process that has
 * NOT been scheduled yet.  This is needed to make kernel threads stay
 * in kernel mode.
 */
void
cpu_set_fork_handler(struct lwp *lp, void (*func)(void *, struct trapframe *),
		     void *arg)
{
	uint64_t *swf = (uint64_t *)lp->lwp_thread->td_sp;

	swf[SWF_X19] = (uint64_t)func;
	swf[SWF_X20] = (uint64_t)arg;
}

/*
 * Prepare a new lwp of an existing process (lwp_create) to start at
 * the given function and stack in user mode.
 */
int
cpu_prepare_lwp(struct lwp *lp, struct lwp_params *params)
{
	struct trapframe *regs = lp->lwp_md.md_regs;

	regs->tf_elr = (uint64_t)params->lwp_func;
	regs->tf_sp = (uint64_t)params->lwp_stack;
	regs->tf_x[0] = (uint64_t)params->lwp_arg;
	regs->tf_lr = 0;		/* returning from lwp_func faults */

	cpu_set_fork_handler(lp,
	    (void (*)(void *, struct trapframe *))generic_lwp_return, lp);
	return (0);
}

/*
 * Reset the registers of the current lwp for a freshly exec'd image:
 * start at (entry) in user mode on (stack), which the image activator
 * has filled with the argument and environment vectors.
 */
void
exec_setregs(u_long entry, u_long stack, u_long ps_strings)
{
	struct thread *td = curthread;
	struct trapframe *regs = td->td_lwp->lwp_md.md_regs;

	bzero(regs, sizeof(*regs));
	regs->tf_elr = entry;
	regs->tf_sp = stack & ~0xFUL;	/* the ABI wants 16 byte alignment */
	regs->tf_x[0] = stack;		/* argument block, for crt0 */
	regs->tf_x[1] = ps_strings;
	regs->tf_spsr = PSR_M_EL0t;	/* user mode, interrupts enabled */

	td->td_pcb->pcb_tpidr_el0 = 0;
	WRITE_SPECIALREG(tpidr_el0, 0);

	/* A new image starts with clean FP/SIMD state. */
	bzero(&td->td_pcb->pcb_save, sizeof(td->td_pcb->pcb_save));
	fpu_restore(&td->td_pcb->pcb_save);
}

/*
 * Set up a new kernel thread to run func(arg) and to call rfunc if
 * func ever returns.
 */
void
cpu_set_thread_handler(thread_t td, void (*rfunc)(void), void *func, void *arg)
{
	td->td_pcb->pcb_x[0] = (register_t)func;
	td->td_pcb->pcb_x[1] = (register_t)arg;
	td->td_switch = cpu_lwkt_switch;
	td->td_sp -= 2 * sizeof(void *);
	((void **)td->td_sp)[0] = cpu_kthread_restore;
	((void **)td->td_sp)[1] = rfunc;
}

/*
 * Terminate the current thread.  The caller must have already acquired
 * the thread's rwlock and placed it on a reap list or otherwise
 * notified a reaper of its existence.  We set a special assembly
 * switch function which releases td_rwlock after it has cleaned up the
 * MMU state and switched out the stack.
 */
void
cpu_thread_exit(void)
{
	curthread->td_switch = cpu_exit_switch;
	curthread->td_flags |= TDF_EXITING;
	lwkt_switch();
	panic("cpu_thread_exit: lwkt_switch() unexpectedly returned");
}

void
cpu_lwp_exit(void)
{
	struct thread *td = curthread;

	td->td_gd->gd_cnt.v_swtch++;

	crit_enter_quick(td);
	if (td->td_flags & TDF_TSLEEPQ)
		tsleep_remove(td);
	lwkt_deschedule_self(td);
	lwkt_remove_tdallq(td);
	cpu_thread_exit();
}

void
cpu_vmspace_alloc(struct vmspace *vm __unused)
{
}

void
cpu_vmspace_free(struct vmspace *vm __unused)
{
}

/*
 * Used by /dev/kmem to determine if we can safely read or write the
 * requested KVA range.
 */
int
kvm_access_check(vm_offset_t saddr, vm_offset_t eaddr, int prot)
{
	vm_offset_t addr;

	if (saddr >= eaddr)
		return (EFAULT);
	for (addr = trunc_page(saddr); addr < eaddr; addr += PAGE_SIZE) {
		if (pmap_kextract(addr) == 0)
			return (EFAULT);
	}
	if (!kernacc((caddr_t)saddr, eaddr - saddr, prot) &&
	    (saddr < DMAP_MIN_ADDRESS || eaddr > DMapMaxAddress))
		return (0);	/* mapped but outside kernel_map: allow */
	return (0);
}
