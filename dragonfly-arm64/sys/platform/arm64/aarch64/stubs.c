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
 * Machine-dependent entry points the kernel requires that are not
 * implemented for arm64 yet.
 *
 * Everything here exists so the kernel links.  The functions fall into
 * three groups: single-cpu versions of SMP hooks (correct while only
 * one cpu runs), hooks for features that do not apply, and genuinely
 * missing functionality, which fails with an error or panics so that
 * reaching it is obvious rather than silently wrong.  This file should
 * shrink to nothing.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/proc.h>
#include <sys/bus.h>
#include <sys/bus_dma.h>
#include <sys/mbuf.h>
#include <sys/linker.h>
#include <sys/ptrace.h>
#include <sys/signalvar.h>
#include <sys/sysmsg.h>
#include <sys/sysproto.h>
#include <sys/kerneldump.h>
#include <sys/tls.h>

#include <vm/vm.h>
#include <vm/pmap.h>

#include <machine/cpu.h>
#include <machine/cpufunc.h>
#include <machine/elf.h>
#include <machine/md_var.h>
#include <machine/reg.h>
#include <machine/smp.h>
#include <machine/vframe.h>

#define	NOT_YET(what)	panic("arm64: " what " is not implemented yet")

/*
 * CPU topology.  Reported flat: every cpu its own single-core,
 * single-thread package.  The device tree's cpu-map node is not read.
 */
void
detect_cpu_topology(void)
{
}

int
get_chip_ID(int cpuid)
{
	return (cpuid);
}

int
get_chip_ID_from_APICID(int apicid)
{
	return (apicid);
}

int
get_core_number_within_chip(int cpuid __unused)
{
	return (0);
}

int
get_logical_CPU_number_within_core(int cpuid __unused)
{
	return (0);
}

/*
 * Not applicable: x86 I/O privilege level and virtual kernel support.
 */
int
cpu_set_iopl(void)
{
	return (ENODEV);
}

int
cpu_clr_iopl(void)
{
	return (ENODEV);
}

void
cpu_vkernel_trap(struct trapframe *frame __unused, int error __unused)
{
	NOT_YET("vkernel support");
}

void
set_vkernel_fp(struct trapframe *frame __unused)
{
	NOT_YET("vkernel support");
}

/*
 * User mode features beyond the basics: the machine-dependent system
 * calls and thread-local storage setup.
 */
int
sys_sysarch(struct sysmsg *sysmsg __unused,
	    const struct sysarch_args *uap __unused)
{
	return (ENOSYS);
}

int
sys_xsyscall(struct sysmsg *sysmsg __unused,
	     const struct nosys_args *uap __unused)
{
	return (ENOSYS);
}

int
sys_set_tls_area(struct sysmsg *sysmsg __unused,
		 const struct set_tls_area_args *uap __unused)
{
	return (ENOSYS);
}

int
sys_get_tls_area(struct sysmsg *sysmsg __unused,
		 const struct get_tls_area_args *uap __unused)
{
	return (ENOSYS);
}

void
set_user_TLS(void)
{
}

int
cpu_sanitize_frame(struct trapframe *frame __unused)
{
	return (EINVAL);
}

int
cpu_sanitize_tls(struct savetls *tls __unused)
{
	return (EINVAL);
}

/*
 * Debugger and procfs access to another process's registers.
 */
int
fill_regs(struct lwp *lp __unused, struct reg *regs __unused)
{
	return (ENOSYS);
}

int
set_regs(struct lwp *lp __unused, struct reg *regs __unused)
{
	return (ENOSYS);
}

int
fill_fpregs(struct lwp *lp __unused, struct fpreg *fpregs __unused)
{
	return (ENOSYS);
}

int
set_fpregs(struct lwp *lp __unused, struct fpreg *fpregs __unused)
{
	return (ENOSYS);
}

int
ptrace_set_pc(struct lwp *lp __unused, unsigned long addr __unused)
{
	return (ENOSYS);
}

int
ptrace_single_step(struct lwp *lp __unused)
{
	return (ENOSYS);
}

int
procfs_read_regs(struct lwp *lp __unused, struct reg *regs __unused)
{
	return (ENOSYS);
}

int
procfs_write_regs(struct lwp *lp __unused, struct reg *regs __unused)
{
	return (ENOSYS);
}

int
procfs_read_fpregs(struct lwp *lp __unused, struct fpreg *fpregs __unused)
{
	return (ENOSYS);
}

int
procfs_write_fpregs(struct lwp *lp __unused, struct fpreg *fpregs __unused)
{
	return (ENOSYS);
}

int
procfs_read_dbregs(struct lwp *lp __unused, struct dbreg *dbregs __unused)
{
	return (ENOSYS);
}

int
procfs_write_dbregs(struct lwp *lp __unused, struct dbreg *dbregs __unused)
{
	return (ENOSYS);
}

/*
 * Kernel modules: relocation processing for loadable objects.
 */
int
elf_reloc(linker_file_t lf __unused, Elf_Addr relocbase __unused,
	  const void *data __unused, int type __unused,
	  elf_lookup_fn lookup __unused)
{
	return (-1);
}

int
elf_reloc_local(linker_file_t lf __unused, Elf_Addr relocbase __unused,
		const void *data __unused, int type __unused,
		elf_lookup_fn lookup __unused)
{
	return (-1);
}

/*
 * Crash dumps.
 */
void
md_dumpsys(struct dumperinfo *di __unused)
{
	kprintf("arm64: kernel dumps are not implemented yet\n");
}

void
dump_add_page(vm_paddr_t pa __unused)
{
}

void
dump_drop_page(vm_paddr_t pa __unused)
{
}

/*
 * No in-kernel debugger yet: say where it would have stopped, and go on
 * as a kernel without DDB does.
 */
void
Debugger(const char *msg)
{
	kprintf("Debugger(\"%s\") called (no in-kernel debugger)\n", msg);
}

/*
 * evdev's keyboard glue (LED and repeat ioctls passed back to a
 * keyboard driver) needs this; there is no keyboard driver yet.
 */
struct keyboard;
int	kbd_ioctl(struct keyboard *kbd, u_long cmd, caddr_t data);

int
kbd_ioctl(struct keyboard *kbd __unused, u_long cmd __unused,
	  caddr_t data __unused)
{
	return (ENODEV);
}
