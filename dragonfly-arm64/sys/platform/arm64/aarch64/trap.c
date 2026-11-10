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
 * Synchronous exception handling.
 *
 * Kernel side: page faults on pageable kernel memory and on user
 * addresses touched by the copyin/copyout family; everything else is
 * fatal.
 *
 * User side: system calls, page faults, and the work done on every
 * return to user mode (userret/userexit, following pc64's trap.c).
 * Other user exceptions are turned into signals.  Not handled yet:
 * single stepping and breakpoints for debuggers.
 *
 * System call convention: number in x8, arguments in x0-x7, further
 * arguments on the stack; results in x0 and x1.  On failure the carry
 * flag is set in the saved PSTATE and x0 holds the errno value.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/proc.h>
#include <sys/signalvar.h>
#include <sys/pioctl.h>
#include <sys/signal2.h>
#include <sys/sysent.h>
#include <sys/sysmsg.h>
#include <sys/syscall.h>
#include <sys/usched.h>
#include <sys/ucred.h>
#include <sys/spinlock2.h>

#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/pmap.h>
#include <vm/vm_kern.h>
#include <vm/vm_map.h>
#include <vm/vm_extern.h>

#include <sys/thread2.h>

#include <cpu/armreg.h>
#include <cpu/frame.h>
#include <machine/cpu.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/pcb.h>
#include <machine/vmparam.h>

static const char *
esr_class_name(u_int ec)
{
	switch (ec) {
	case EXCP_UNKNOWN:	return ("unknown reason");
	case EXCP_FP_SIMD:	return ("FP/SIMD access");
	case EXCP_ILL_STATE:	return ("illegal execution state");
	case EXCP_SVC64:	return ("system call");
	case EXCP_MSR:		return ("trapped system register access");
	case EXCP_INSN_ABORT_L:	return ("instruction abort (user)");
	case EXCP_INSN_ABORT:	return ("instruction abort (kernel)");
	case EXCP_PC_ALIGN:	return ("PC alignment fault");
	case EXCP_DATA_ABORT_L:	return ("data abort (user)");
	case EXCP_DATA_ABORT:	return ("data abort (kernel)");
	case EXCP_SP_ALIGN:	return ("SP alignment fault");
	case EXCP_SERROR:	return ("SError");
	case EXCP_BRK:		return ("breakpoint (BRK)");
	default:		return ("unhandled exception class");
	}
}

static void
dump_frame(struct trapframe *frame)
{
	int i;

	kprintf("  elr  0x%016jx  spsr 0x%08jx\n", (uintmax_t)frame->tf_elr,
	    (uintmax_t)frame->tf_spsr);
	kprintf("  esr  0x%08x          far  0x%016jx\n", frame->tf_esr,
	    (uintmax_t)frame->tf_far);
	kprintf("  sp   0x%016jx  lr   0x%016jx\n", (uintmax_t)frame->tf_sp,
	    (uintmax_t)frame->tf_lr);
	for (i = 0; i < 30; i += 2) {
		kprintf("  x%-2d  0x%016jx  x%-2d  0x%016jx\n",
		    i, (uintmax_t)frame->tf_x[i],
		    i + 1, (uintmax_t)frame->tf_x[i + 1]);
	}
}

/*
 * Handle a translation, access or permission fault taken in the
 * kernel.  Returns 0 if the faulting instruction can be restarted (or
 * execution was redirected to a recovery handler).
 */
static int
trap_pfault(struct trapframe *frame, u_int ec)
{
	struct thread *td = curthread;
	vm_offset_t va = trunc_page(frame->tf_far);
	vm_map_t map;
	vm_prot_t ftype;
	int fault_flags;
	int rv;

	if (ec == EXCP_INSN_ABORT)
		ftype = VM_PROT_EXECUTE;
	else if (frame->tf_esr & ISS_DATA_WnR)
		ftype = VM_PROT_WRITE;
	else
		ftype = VM_PROT_READ;

	if (va >= KvaStart && va < KvaEnd) {
		map = kernel_map;
	} else if (va < VM_MAX_USER_ADDRESS && td->td_lwp != NULL) {
		map = &td->td_lwp->lwp_vmspace->vm_map;
	} else {
		map = NULL;
	}

	/*
	 * Resolving the fault can block, so it can only be attempted
	 * from thread context with interrupts as the faulting code had
	 * them.
	 */
	if (map != NULL && td->td_gd->gd_intr_nesting_level == 0 &&
	    td->td_critcount == 0) {
		if ((frame->tf_spsr & PSR_I) == 0)
			cpu_enable_intr();
		fault_flags = (ftype & VM_PROT_WRITE) ?
		    VM_FAULT_DIRTY : VM_FAULT_NORMAL;
		rv = vm_fault(map, va, ftype, fault_flags);
		cpu_disable_intr();
		if (rv == KERN_SUCCESS)
			return (0);
	}

	/*
	 * copyin/copyout and friends arm a recovery address before they
	 * touch user memory.
	 */
	if (td->td_pcb != NULL && td->td_pcb->pcb_onfault != NULL &&
	    td->td_pcb->pcb_onfault_sp == (register_t)frame->tf_sp) {
		frame->tf_elr = (uint64_t)td->td_pcb->pcb_onfault;
		return (0);
	}
	return (-1);
}

void
trap(struct trapframe *frame)
{
	u_int ec = ESR_ELx_EC(frame->tf_esr);

	switch (ec) {
	case EXCP_DATA_ABORT:
	case EXCP_INSN_ABORT:
		if (trap_pfault(frame, ec) == 0)
			return;
		break;
	default:
		break;
	}
	trap_fatal(frame, 1);
}

void
trap_fatal(struct trapframe *frame, int from_el1)
{
	u_int ec = ESR_ELx_EC(frame->tf_esr);

	cpu_disable_intr();
	kprintf("\n\nFatal %s exception: %s (EC 0x%x)\n",
	    from_el1 ? "kernel" : "user", esr_class_name(ec), ec);
	kprintf("  cpu %d, thread %p (%s)\n", mycpu->gd_cpuid, curthread,
	    curthread->td_comm);
	dump_frame(frame);
	if (!from_el1)
		panic("exception from user mode: not supported yet");
	panic("%s", esr_class_name(ec));
}

/*
 * The three steps that bracket every excursion into the kernel on
 * behalf of a user process; see pc64's trap.c for the full story.
 *
 * userenter() - called on entry from user mode.  Arrange for our LWKT
 * priority to be raised lazily if we block, and pick up any change of
 * the process credentials.
 */
static __inline void
userenter(struct thread *curtd, struct proc *curp)
{
	struct ucred *ocred;
	struct ucred *ncred;

	curtd->td_release = lwkt_passive_release;

	if (__predict_false(curtd->td_ucred != curp->p_ucred)) {
		spin_lock(&curp->p_spin);
		ncred = crhold(curp->p_ucred);
		spin_unlock(&curp->p_spin);
		ocred = curtd->td_ucred;
		curtd->td_ucred = ncred;
		if (ocred)
			crfree(ocred);
	}
}

/*
 * userret() - handle signals, stops and other pending conditions
 * before going back to user mode.
 */
static void
userret(struct lwp *lp, struct trapframe *frame __unused)
{
	struct proc *p = lp->lwp_proc;
	int sig;
	int ptok;

recheck:
	/* Post any pending upcalls, cpu migration requests and the like. */
	if (lp->lwp_mpflags & LWP_MP_URETMASK)
		lwpuserret(lp);

	/* Block here if we are in a stopped state. */
	if (__predict_false(STOPLWP(p, lp))) {
		lwkt_gettoken(&p->p_token);
		tstop();
		lwkt_reltoken(&p->p_token);
		goto recheck;
	}

	/* Post any pending signals. */
	if (__predict_false((sig = CURSIG_LCK_TRACE(lp, &ptok)) != 0)) {
		postsig(sig, ptok);
		goto recheck;
	}

	/* A sigsuspend-style temporary mask is restored on the way out. */
	if (__predict_false(lp->lwp_flags & LWP_OLDMASK)) {
		lp->lwp_flags &= ~LWP_OLDMASK;
		lp->lwp_sigmask = lp->lwp_oldsigmask;
		goto recheck;
	}
}

/*
 * userexit() - the last thing before returning to user mode: drop the
 * kernel priority boost and (re)acquire the right to run as the
 * current user process on this cpu, which is where the user scheduler
 * gets to switch us out.
 */
static void
userexit(struct lwp *lp)
{
	struct thread *td = lp->lwp_thread;

	while (__predict_false(STOPLWP(lp->lwp_proc, lp))) {
		lwkt_gettoken(&lp->lwp_proc->p_token);
		tstop();
		lwkt_reltoken(&lp->lwp_proc->p_token);
	}
	lwkt_passive_recover(td);
	lp->lwp_proc->p_usched->acquire_curproc(lp);
}

/*
 * A system call from user mode.
 */
static void
syscall2(struct trapframe *frame)
{
	struct thread *td = curthread;
	struct proc *p = td->td_proc;
	struct lwp *lp = td->td_lwp;
	struct sysent *callp;
	struct sysmsg sysmsg;
	union sysunion *argp;
	const int regcnt = 8;	/* number of args passed in registers */
	int crit_count = td->td_critcount;
	u_int code;
	int error;
	int narg;

	mycpu->gd_cnt.v_syscall++;
	userenter(td, p);	/* lazy raise our priority */

	KASSERT(lp->lwp_md.md_regs == frame,
		("Frame mismatch %p %p", lp->lwp_md.md_regs, frame));

	code = (u_int)frame->tf_x[8];
	if (code >= p->p_sysent->sv_size)
		code = SYS___nosys;
	callp = &p->p_sysent->sv_table[code];
	narg = callp->sy_narg;

	/*
	 * The first eight arguments are already laid out as an array of
	 * registers in the trapframe; any more are on the user stack.
	 */
	argp = (union sysunion *)&frame->tf_x[0];
	if (__predict_false(narg > regcnt)) {
		register_t *argsdst = (void *)&sysmsg.extargs;

		bcopy(argp, argsdst, sizeof(register_t) * regcnt);
		error = copyin((caddr_t)frame->tf_sp, &argsdst[regcnt],
			       (narg - regcnt) * sizeof(register_t));
		argp = (void *)argsdst;
		if (error)
			goto bad;
	}

	/*
	 * Default return value is 0 (will be copied to x0), with x1
	 * preserved for calls that only return one value.
	 */
	sysmsg.sysmsg_fds[0] = 0;
	sysmsg.sysmsg_fds[1] = frame->tf_x[1];
	sysmsg.sysmsg_frame = frame;

	STOPEVENT(p, S_SCE, narg);	/* MP aware */

	error = (*callp->sy_call)(&sysmsg, argp);

	if (__predict_true(error == 0)) {
		/* The process and lwp may have changed across vfork/exec. */
		p = curproc;
		lp = curthread->td_lwp;
		/*
		 * A successful execve() does not return: exec_setregs()
		 * built a whole new frame, and x0, which would carry the
		 * return value, is the new image's first argument.
		 */
		if (code != SYS_execve) {
			frame->tf_x[0] = sysmsg.sysmsg_fds[0];
			frame->tf_x[1] = sysmsg.sysmsg_fds[1];
			frame->tf_spsr &= ~PSR_C;
		}
	} else if (error == ERESTART) {
		/* Back up over the 4-byte SVC so it is executed again. */
		frame->tf_elr -= 4;
	} else if (error == EJUSTRETURN) {
		/* do nothing */
	} else {
bad:
		frame->tf_x[0] = error;
		frame->tf_spsr |= PSR_C;
	}

	userret(lp, frame);
	STOPEVENT(p, S_SCX, code);
	userexit(lp);

	KASSERT(crit_count == td->td_critcount,
		("syscall: critical section count mismatch! %d/%d in %s "
		 "sysno=%d", crit_count, td->td_critcount, td->td_comm, code));
	KASSERT(&td->td_toks_base == td->td_toks_stop,
		("syscall: %ld extra tokens held after trap! syscall %p",
		 td->td_toks_stop - &td->td_toks_base, callp->sy_call));
}

/*
 * A page fault from user mode.  Returns the signal to deliver, or 0
 * if the fault was resolved.
 */
static int
user_pfault(struct trapframe *frame, u_int ec)
{
	struct lwp *lp = curthread->td_lwp;
	vm_offset_t va = trunc_page(frame->tf_far);
	vm_prot_t ftype;
	int rv;

	if (ec == EXCP_INSN_ABORT_L)
		ftype = VM_PROT_EXECUTE;
	else if (frame->tf_esr & ISS_DATA_WnR)
		ftype = VM_PROT_WRITE;
	else
		ftype = VM_PROT_READ;

	if (va >= VM_MAX_USER_ADDRESS)
		return (SIGSEGV);
	rv = vm_fault(&lp->lwp_vmspace->vm_map, va, ftype,
	    (ftype & VM_PROT_WRITE) ? VM_FAULT_DIRTY : VM_FAULT_NORMAL);
	if (rv == KERN_SUCCESS)
		return (0);
	return (rv == KERN_PROTECTION_FAILURE ? SIGBUS : SIGSEGV);
}

/*
 * Synchronous exception from user mode, called from the vector with
 * interrupts masked.
 */
void
trap_el0(struct trapframe *frame)
{
	struct thread *td = curthread;
	struct lwp *lp = td->td_lwp;
	u_int ec = ESR_ELx_EC(frame->tf_esr);
	int sig = 0;
	int code = 0;

	cpu_enable_intr();

	if (ec == EXCP_SVC64) {
		syscall2(frame);
		return;
	}

	userenter(td, td->td_proc);
	switch (ec) {
	case EXCP_DATA_ABORT_L:
	case EXCP_INSN_ABORT_L:
		sig = user_pfault(frame, ec);
		code = (sig == SIGBUS) ? BUS_ADRERR : SEGV_MAPERR;
		break;
	case EXCP_PC_ALIGN:
	case EXCP_SP_ALIGN:
		sig = SIGBUS;
		code = BUS_ADRALN;
		break;
	case EXCP_BRK:
	case EXCP_BRKPT_EL0:
	case EXCP_SOFTSTP_EL0:
		sig = SIGTRAP;
		code = TRAP_BRKPT;
		break;
	case EXCP_TRAP_FP:
		sig = SIGFPE;
		code = FPE_FLTINV;
		break;
	default:
		sig = SIGILL;
		code = ILL_ILLOPC;
		break;
	}
	if (sig != 0) {
		static pid_t lastpid;
		static int lastticks;

		/*
		 * Say why, but not over and over: a process that ignores
		 * the signal (init, once its executable is gone during
		 * shutdown) faults in a tight loop.
		 */
		if ((bootverbose || sig != SIGTRAP) &&
		    (td->td_proc->p_pid != lastpid ||
		     (u_int)(ticks - lastticks) > (u_int)(10 * hz))) {
			lastpid = td->td_proc->p_pid;
			lastticks = ticks;
			kprintf("pid %d (%s): %s at pc 0x%jx, addr 0x%jx, "
			    "signal %d\n", td->td_proc->p_pid,
			    td->td_proc->p_comm, esr_class_name(ec),
			    (uintmax_t)frame->tf_elr,
			    (uintmax_t)frame->tf_far, sig);
		}
		trapsignal(lp, sig, code);
	}
	userret(lp, frame);
	userexit(lp);
}

/*
 * Last stop before the vector code restores the trapframe and returns
 * to user mode: service asynchronous requests (ASTs) that were posted
 * against this cpu while we were in the kernel, and come back with
 * interrupts masked once there are none left.  This is what pc64 does
 * in doreti.
 */
void
arm64_user_return(struct trapframe *frame)
{
	struct thread *td = curthread;
	struct lwp *lp = td->td_lwp;
	struct globaldata *gd;

	for (;;) {
		cpu_disable_intr();
		gd = td->td_gd;
		if ((gd->gd_reqflags & RQF_AST_MASK) == 0)
			break;
		atomic_clear_int(&gd->gd_reqflags,
		    RQF_AST_SIGNAL | RQF_AST_OWEUPC);
		cpu_enable_intr();

		userenter(td, lp->lwp_proc);
		if (lwkt_resched_wanted())
			lwkt_switch();
		userret(lp, frame);
		userexit(lp);
	}
}

/*
 * Called from fork_trampoline in a newly created lwp, just before it
 * enters user mode for the first time.
 */
void
generic_lwp_return(struct lwp *lp, struct trapframe *frame)
{
	struct proc *p = lp->lwp_proc;

	/*
	 * Check for exit-race.  If one lwp exits the process concurrent
	 * with another lwp creating a new thread, the two operations
	 * may cross each other resulting in the newly-created lwp not
	 * receiving a KILL signal.
	 */
	if (p->p_flags & P_WEXIT)
		lwpsignal(p, lp, SIGKILL);

	lwkt_setpri_self(TDPRI_USER_NORM);
	userenter(lp->lwp_thread, p);
	userret(lp, frame);
	lp->lwp_flags |= LWP_PASSIVE_ACQ;
	userexit(lp);
	lp->lwp_flags &= ~LWP_PASSIVE_ACQ;
}

/*
 * Simplified back end of fork: the child returns 0 from the system
 * call, with the carry flag (error indication) clear.
 */
void
fork_return(struct lwp *lp, struct trapframe *frame)
{
	frame->tf_x[0] = 0;		/* Child returns zero */
	frame->tf_x[1] = 1;
	frame->tf_spsr &= ~PSR_C;	/* success */
	generic_lwp_return(lp, frame);
}
