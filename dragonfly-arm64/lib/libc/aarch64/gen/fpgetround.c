/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#include <sys/types.h>
#include <ieeefp.h>

fp_rnd_t
fpgetround(void)
{
	uint64_t fpcr;

	__asm __volatile("mrs %0, fpcr" : "=r" (fpcr));
	return ((fp_rnd_t)(fpcr & (3 << 22)));
}
