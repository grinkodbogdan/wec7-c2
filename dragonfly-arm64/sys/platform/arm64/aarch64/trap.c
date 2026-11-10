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
 * Synchronous exception and interrupt dispatch.
 *
 * Bring-up version: handles what the boot self-tests need (BRK and
 * recoverable kernel data aborts) and reports everything else as
 * fatal.  The real trap() additionally has to route EL0 exceptions:
 * SVC to syscall2(), aborts to vm_fault(), FP traps to the lazy FPU
 * code, and deliver signals on the way back to userland.
 */

#include <sys/types.h>
#include <sys/_null.h>
#include <cpu/armreg.h>
#include <cpu/cpufunc.h>
#include <cpu/frame.h>
#include <machine/md_var.h>

int	kprintf(const char *fmt, ...) __printflike(1, 2);

/*
 * If non-zero, a kernel data abort resumes here instead of panicing.
 * Equivalent to pcb_onfault on x86_64, where it lives in the pcb.
 */
u_long	arm64_onfault;

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

	kprintf("  elr  0x%016lx  spsr 0x%08lx\n", frame->tf_elr,
	    frame->tf_spsr);
	kprintf("  esr  0x%08x          far  0x%016lx\n", frame->tf_esr,
	    frame->tf_far);
	kprintf("  sp   0x%016lx  lr   0x%016lx\n", frame->tf_sp,
	    frame->tf_lr);
	for (i = 0; i < 30; i += 2) {
		kprintf("  x%-2d  0x%016lx  x%-2d  0x%016lx\n",
		    i, frame->tf_x[i], i + 1, frame->tf_x[i + 1]);
	}
}

void
trap(struct trapframe *frame)
{
	u_int ec = ESR_ELx_EC(frame->tf_esr);

	switch (ec) {
	case EXCP_BRK:
		kprintf("trap: BRK #%u at 0x%lx, resuming\n",
		    frame->tf_esr & 0xffff, frame->tf_elr);
		frame->tf_elr += 4;
		return;
	case EXCP_DATA_ABORT:
		if (arm64_onfault != 0) {
			frame->tf_elr = arm64_onfault;
			arm64_onfault = 0;
			return;
		}
		break;
	default:
		break;
	}
	trap_fatal(frame, 1);
}

void
trap_fatal(struct trapframe *frame, int from_el1)
{
	kprintf("\nFatal %s exception: %s (EC 0x%x)\n",
	    from_el1 ? "kernel" : "user",
	    esr_class_name(ESR_ELx_EC(frame->tf_esr)),
	    (u_int)ESR_ELx_EC(frame->tf_esr));
	dump_frame(frame);
	kprintf("panic: unhandled exception\n");
	cpu_poweroff();
}

void
arm64_intr(struct trapframe *frame __unused)
{
	int iar;

	while ((iar = gic_ack()) != -1) {
		if ((iar & 0x3ff) == arm64_bootinfo.bi_timer_irq)
			clock_intr();
		else
			kprintf("stray interrupt %d\n", iar & 0x3ff);
		gic_eoi(iar);
	}
}
