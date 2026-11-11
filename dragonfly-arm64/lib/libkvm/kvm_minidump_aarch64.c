/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * Minidumps on aarch64: not implemented yet (see kvm_aarch64.c).
 */
#include <sys/user.h>	/* MUST BE FIRST */
#include <sys/param.h>
#include <stdlib.h>
#include <nlist.h>

#include <limits.h>

#include "kvm.h"
#include "kvm_private.h"

void
_kvm_minidump_freevtop(kvm_t *kd __unused)
{
}

int
_kvm_minidump_initvtop(kvm_t *kd)
{
	_kvm_err(kd, kd->program, "minidumps are not supported on aarch64 yet");
	return (-1);
}

int
_kvm_minidump_kvatop(kvm_t *kd, u_long va __unused, off_t *pa __unused)
{
	_kvm_err(kd, 0, "kvm_kvatop: not supported on aarch64 yet");
	return (0);
}
