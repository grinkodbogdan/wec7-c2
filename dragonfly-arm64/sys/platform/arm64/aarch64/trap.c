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
 * Only exceptions taken from the kernel itself are handled: kernel
 * page faults (pageable kernel memory, and user addresses touched by
 * the copyin/copyout family) and fatal conditions.  The user-mode half
 * of trap() on pc64 (system calls via syscall2(), user faults, signal
 * delivery through userret(), FP traps) has no counterpart here yet.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/proc.h>

#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/pmap.h>
#include <vm/vm_kern.h>
#include <vm/vm_map.h>
#include <vm/vm_extern.h>

#include <sys/thread2.h>

#include <cpu/armreg.h>
#include <cpu/frame.h>
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
 * A forked process tried to return to user mode.
 */
void
arm64_usermode_unimplemented(struct trapframe *frame)
{
	kprintf("\nReturn to user mode requested (pc 0x%jx)\n",
	    (uintmax_t)frame->tf_elr);
	panic("arm64: user mode is not implemented yet");
}

/*
 * Called from fork_trampoline in a newly created lwp, just before it
 * would enter user mode for the first time.
 *
 * XXX The pc64 version goes through userenter()/userret() here to
 * deliver pending signals and ASTs; that machinery belongs to the
 * user-mode half of this file and is still missing.
 */
void
generic_lwp_return(struct lwp *lp __unused, struct trapframe *frame __unused)
{
	lwkt_setpri_self(TDPRI_USER_NORM);
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
