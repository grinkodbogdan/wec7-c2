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
 * ELF image activation glue: the system call vector and "brand" that
 * tell kern/imgact_elf.c which executables this kernel runs natively.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/exec.h>
#include <sys/imgact.h>
#include <sys/linker.h>
#include <sys/sysent.h>
#include <sys/imgact_elf.h>
#include <sys/syscall.h>
#include <sys/signalvar.h>
#include <sys/vnode.h>

#include <machine/elf.h>
#include <machine/md_var.h>

static struct sysentvec elf64_dragonfly_sysvec = {
	.sv_size	= SYS_MAXSYSCALL,
	.sv_table	= sysent,
	.sv_sigsize	= 0,
	.sv_sigtbl	= NULL,
	.sv_errsize	= 0,
	.sv_errtbl	= NULL,
	.sv_transtrap	= NULL,
	.sv_fixup	= __elfN(dragonfly_fixup),
	.sv_sendsig	= sendsig,
	.sv_sigcode	= sigcode,
	.sv_szsigcode	= &szsigcode,
	.sv_name	= "DragonFly ELF64",
	.sv_coredump	= __elfN(coredump),
	.sv_imgact_try	= NULL,
	.sv_minsigstksz	= MINSIGSTKSZ,
};

static Elf64_Brandinfo dragonfly_brand_info = {
	.brand		= ELFOSABI_NONE,
	.machine	= EM_AARCH64,
	.compat_3_brand	= "DragonFly",
	.emul_path	= NULL,
	.interp_path	= "/libexec/ld-elf.so.2",
	.sysvec		= &elf64_dragonfly_sysvec,
	.interp_newpath	= NULL,
	.flags		= BI_CAN_EXEC_DYN | BI_BRAND_NOTE,
	.brand_note	= &elf64_dragonfly_brandnote,
};

SYSINIT(elf64, SI_SUB_EXEC, SI_ORDER_FIRST,
	(sysinit_cfunc_t) elf64_insert_brand_entry,
	&dragonfly_brand_info);
