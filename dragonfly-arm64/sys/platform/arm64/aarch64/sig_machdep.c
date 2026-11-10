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
 * Signal delivery to user handlers.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/proc.h>
#include <sys/signalvar.h>
#include <sys/sysent.h>
#include <sys/sysmsg.h>
#include <sys/sysproto.h>
#include <sys/exec.h>
#include <sys/ucontext.h>

#include <vm/vm.h>
#include <vm/vm_param.h>

#include <sys/thread2.h>

#include <cpu/armreg.h>
#include <cpu/frame.h>
#include <cpu/fpu.h>
#include <machine/md_var.h>
#include <machine/pcb.h>
#include <machine/sigframe.h>
#include <machine/vmparam.h>

CTASSERT(sizeof(struct trapframe) ==
    __offsetof(mcontext_t, mc_len) - __offsetof(mcontext_t, mc_sp));
CTASSERT(sizeof(struct savefpu) <= sizeof(((mcontext_t *)0)->mc_fpregs));

/*
 * Send an interrupt to process.
 *
 * Stack is set up to allow the signal trampoline (sigcode, in
 * exception.S) to call the handler and then sigreturn(2) with the
 * saved context, which resumes the interrupted code as if nothing
 * happened.  The frame holds the handler address, the full register
 * and FP/SIMD state, the signal mask to restore and, for SA_SIGINFO
 * handlers, a siginfo.
 */
void
sendsig(sig_t catcher, int sig, sigset_t *mask, u_long code)
{
	struct lwp *lp = curthread->td_lwp;
	struct proc *p = lp->lwp_proc;
	struct trapframe *regs;
	struct sigacts *psp = p->p_sigacts;
	struct sigframe sf, *sfp;
	int oonstack;
	char *sp;

	regs = lp->lwp_md.md_regs;
	oonstack = (lp->lwp_sigstk.ss_flags & SS_ONSTACK) ? 1 : 0;

	/* Save user context */
	bzero(&sf, sizeof(struct sigframe));
	sf.sf_uc.uc_sigmask = *mask;
	sf.sf_uc.uc_stack = lp->lwp_sigstk;
	sf.sf_uc.uc_mcontext.mc_onstack = oonstack;
	bcopy(regs, &sf.sf_uc.uc_mcontext.mc_sp, sizeof(struct trapframe));
	sf.sf_uc.uc_mcontext.mc_tpidr = READ_SPECIALREG(tpidr_el0);

	/* Make the size of the saved context available to userland */
	sf.sf_uc.uc_mcontext.mc_len = sizeof(sf.sf_uc.uc_mcontext);

	/* The handler is free to use FP/SIMD: save what was live. */
	fpu_save((struct savefpu *)sf.sf_uc.uc_mcontext.mc_fpregs);
	sf.sf_uc.uc_mcontext.mc_fpformat = _MC_FPFMT_VFP;
	sf.sf_uc.uc_mcontext.mc_ownedfp = _MC_FPOWNED_FPU;

	/* Allocate and validate space for the signal handler context. */
	if ((lp->lwp_flags & LWP_ALTSTACK) != 0 && !oonstack &&
	    SIGISMEMBER(psp->ps_sigonstack, sig)) {
		sp = (char *)lp->lwp_sigstk.ss_sp + lp->lwp_sigstk.ss_size -
		    sizeof(struct sigframe);
		lp->lwp_sigstk.ss_flags |= SS_ONSTACK;
	} else {
		sp = (char *)regs->tf_sp - sizeof(struct sigframe);
	}

	/* Align to 16 bytes */
	sfp = (struct sigframe *)((intptr_t)sp & ~(intptr_t)0xF);

	/* Build the argument list for the signal handler. */
	regs->tf_x[0] = sig;				/* argument 1 */
	regs->tf_x[2] = (register_t)&sfp->sf_uc;	/* argument 3 */
	regs->tf_x[3] = (register_t)regs->tf_far;	/* argument 4 */
	if (SIGISMEMBER(psp->ps_siginfo, sig)) {
		/* Signal handler installed with SA_SIGINFO. */
		regs->tf_x[1] = (register_t)&sfp->sf_si; /* argument 2 */
		sf.sf_ahu.sf_action = (__siginfohandler_t *)catcher;

		/* fill siginfo structure */
		sf.sf_si.si_signo = sig;
		sf.sf_si.si_pid = psp->ps_frominfo[sig].pid;
		sf.sf_si.si_uid = psp->ps_frominfo[sig].uid;
		sf.sf_si.si_code = code;
		sf.sf_si.si_addr = (void *)regs->tf_far;
	} else {
		/* Old FreeBSD-style arguments. */
		regs->tf_x[1] = (register_t)code;	/* argument 2 */
		sf.sf_ahu.sf_handler = catcher;
	}

	/* Copy the sigframe out to the user's stack. */
	if (copyout(&sf, sfp, sizeof(struct sigframe)) != 0) {
		/*
		 * Something is wrong with the stack pointer.
		 * ...Kill the process.
		 */
		sigexit(lp, SIGILL);
	}

	regs->tf_sp = (register_t)sfp;
	regs->tf_elr = trunc_page64(PS_STRINGS - *(p->p_sysent->sv_szsigcode));
	regs->tf_elr -= SZSIGCODE_EXTRA_BYTES;
	regs->tf_spsr &= ~PSR_AARCH32;
}

/*
 * sigreturn(ucontext_t *sigcntxp)
 *
 * System call to cleanup state after a signal has been taken.  Reset
 * signal mask and stack state from context left by sendsig (above).
 * Return to previous pc and psl as specified by context left by
 * sendsig.  Check carefully to make sure that the user has not
 * modified the state to gain improper privileges.
 */
int
sys_sigreturn(struct sysmsg *sysmsg __unused, const struct sigreturn_args *uap)
{
	struct lwp *lp = curthread->td_lwp;
	struct trapframe *regs;
	ucontext_t uc;
	ucontext_t *ucp;
	int error;

	/*
	 * We have to copy the information into kernel space so userland
	 * can't modify it while we are sniffing it.
	 */
	regs = lp->lwp_md.md_regs;
	error = copyin(uap->sigcntxp, &uc, sizeof(uc));
	if (error)
		return (error);
	ucp = &uc;

	/*
	 * The saved processor state must describe 64-bit user mode with
	 * no exceptions masked; anything else would let the process
	 * resume with kernel privilege.
	 */
	if ((ucp->uc_mcontext.mc_spsr &
	    (PSR_M_MASK | PSR_AARCH32 | PSR_DAIF)) != PSR_M_EL0t)
		return (EINVAL);

	bcopy(&ucp->uc_mcontext.mc_sp, regs, sizeof(struct trapframe));
	WRITE_SPECIALREG(tpidr_el0, ucp->uc_mcontext.mc_tpidr);

	crit_enter();
	if (ucp->uc_mcontext.mc_ownedfp == _MC_FPOWNED_FPU)
		fpu_restore((struct savefpu *)ucp->uc_mcontext.mc_fpregs);

	if (ucp->uc_mcontext.mc_onstack & 1)
		lp->lwp_sigstk.ss_flags |= SS_ONSTACK;
	else
		lp->lwp_sigstk.ss_flags &= ~SS_ONSTACK;

	lp->lwp_sigmask = ucp->uc_sigmask;
	SIG_CANTMASK(lp->lwp_sigmask);
	crit_exit();

	return (EJUSTRETURN);
}
