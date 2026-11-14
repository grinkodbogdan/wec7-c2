/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * Machine part of <fenv.h> for aarch64.  Values follow FreeBSD/arm64:
 * exception flags are the FPSR cumulative bits, rounding modes the
 * FPCR RMode field shifted down by 22.
 */

#ifndef	_ARM64_FENV_H_
#define	_ARM64_FENV_H_

#include <sys/cdefs.h>
#include <machine/stdint.h>

#define	FE_INVALID	0x00000001
#define	FE_DIVBYZERO	0x00000002
#define	FE_OVERFLOW	0x00000004
#define	FE_UNDERFLOW	0x00000008
#define	FE_INEXACT	0x00000010
#define	FE_ALL_EXCEPT	(FE_DIVBYZERO | FE_INEXACT | \
			 FE_INVALID | FE_OVERFLOW | FE_UNDERFLOW)

#define	FE_TONEAREST	0x0
#define	FE_UPWARD	0x1
#define	FE_DOWNWARD	0x2
#define	FE_TOWARDZERO	0x3

/* The high 32 bits hold FPCR, the low 32 FPSR. */
typedef	__uint64_t	fenv_t;
typedef	__uint64_t	fexcept_t;

__BEGIN_DECLS
extern fenv_t	__fe_dfl_env;
__END_DECLS
#define	FE_DFL_ENV	((const fenv_t *)&__fe_dfl_env)

#endif	/* !_ARM64_FENV_H_ */
