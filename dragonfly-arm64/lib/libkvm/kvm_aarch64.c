/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * Kernel virtual to physical translation for crash dumps on aarch64:
 * not implemented yet.  Live-system use of libkvm (ps, top, ...) goes
 * through sysctl and never translates addresses, so this only stops
 * reading crash dumps, which the arm64 kernel cannot write yet anyway.
 */
#include <sys/user.h>	/* MUST BE FIRST */
#include <sys/param.h>
#include <stdlib.h>
#include <nlist.h>

#include <limits.h>

#include "kvm.h"
#include "kvm_private.h"

void
_kvm_freevtop(kvm_t *kd)
{
	if (kd->rawdump == 0)
		_kvm_minidump_freevtop(kd);
}

int
_kvm_initvtop(kvm_t *kd)
{
	_kvm_err(kd, kd->program, "crash dumps are not supported on aarch64 yet");
	return (-1);
}

int
_kvm_kvatop(kvm_t *kd, u_long va __unused, off_t *pa __unused)
{
	_kvm_err(kd, 0, "kvm_kvatop: not supported on aarch64 yet");
	return (0);
}
