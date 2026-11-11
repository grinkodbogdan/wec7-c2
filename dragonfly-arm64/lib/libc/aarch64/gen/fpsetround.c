/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <sys/types.h>
#include <ieeefp.h>

fp_rnd_t
fpsetround(fp_rnd_t rnd)
{
	uint64_t old, new;

	__asm __volatile("mrs %0, fpcr" : "=r" (old));
	new = (old & ~(3UL << 22)) | ((uint64_t)rnd & (3UL << 22));
	__asm __volatile("msr fpcr, %0" : : "r" (new));
	return ((fp_rnd_t)(old & (3 << 22)));
}
