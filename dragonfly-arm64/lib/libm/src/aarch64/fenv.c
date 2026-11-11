/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * Floating-point environment for aarch64, after FreeBSD/arm64's.
 * fenv_t holds FPCR in its high 32 bits and FPSR in its low 32.
 * Exception flags are the FPSR cumulative bits (FE_* values); the
 * matching trap-enable bits in FPCR are the same bits shifted left 8.
 * Rounding modes are FPCR.RMode (bits 23:22).
 */
#include <sys/types.h>
#include <fenv.h>

#define	ROUND_SHIFT	22
#define	ROUND_MASK	(FE_TONEAREST | FE_UPWARD | FE_DOWNWARD | FE_TOWARDZERO)
#define	ENABLE_SHIFT	8

fenv_t __fe_dfl_env = 0;

#define	mrs_fpcr(r)	__asm __volatile("mrs %0, fpcr" : "=r" (r))
#define	msr_fpcr(r)	__asm __volatile("msr fpcr, %0" : : "r" (r))
#define	mrs_fpsr(r)	__asm __volatile("mrs %0, fpsr" : "=r" (r))
#define	msr_fpsr(r)	__asm __volatile("msr fpsr, %0" : : "r" (r))

int
feclearexcept(int excepts)
{
	uint64_t fpsr;

	mrs_fpsr(fpsr);
	fpsr &= ~(uint64_t)(excepts & FE_ALL_EXCEPT);
	msr_fpsr(fpsr);
	return (0);
}

int
fegetexceptflag(fexcept_t *flagp, int excepts)
{
	uint64_t fpsr;

	mrs_fpsr(fpsr);
	*flagp = fpsr & excepts & FE_ALL_EXCEPT;
	return (0);
}

int
fesetexceptflag(const fexcept_t *flagp, int excepts)
{
	uint64_t fpsr;

	excepts &= FE_ALL_EXCEPT;
	mrs_fpsr(fpsr);
	fpsr &= ~(uint64_t)excepts;
	fpsr |= *flagp & excepts;
	msr_fpsr(fpsr);
	return (0);
}

int
feraiseexcept(int excepts)
{
	fexcept_t ex = excepts & FE_ALL_EXCEPT;

	/* Setting the flags does not trap; good enough, as on FreeBSD. */
	fesetexceptflag(&ex, excepts);
	return (0);
}

int
fetestexcept(int excepts)
{
	uint64_t fpsr;

	mrs_fpsr(fpsr);
	return (fpsr & excepts & FE_ALL_EXCEPT);
}

int
fegetround(void)
{
	uint64_t fpcr;

	mrs_fpcr(fpcr);
	return ((fpcr >> ROUND_SHIFT) & ROUND_MASK);
}

int
fesetround(int round)
{
	uint64_t fpcr;

	if (round & ~ROUND_MASK)
		return (-1);
	mrs_fpcr(fpcr);
	fpcr &= ~((uint64_t)ROUND_MASK << ROUND_SHIFT);
	fpcr |= (uint64_t)round << ROUND_SHIFT;
	msr_fpcr(fpcr);
	return (0);
}

int
fegetenv(fenv_t *envp)
{
	uint64_t fpcr, fpsr;

	mrs_fpcr(fpcr);
	mrs_fpsr(fpsr);
	*envp = (fpcr << 32) | (fpsr & 0xffffffff);
	return (0);
}

int
feholdexcept(fenv_t *envp)
{
	uint64_t fpcr, fpsr;

	fegetenv(envp);
	mrs_fpcr(fpcr);
	fpcr &= ~((uint64_t)FE_ALL_EXCEPT << ENABLE_SHIFT);
	msr_fpcr(fpcr);
	mrs_fpsr(fpsr);
	fpsr &= ~(uint64_t)FE_ALL_EXCEPT;
	msr_fpsr(fpsr);
	return (0);
}

int
fesetenv(const fenv_t *envp)
{
	uint64_t fpcr = *envp >> 32, fpsr = *envp & 0xffffffff;

	msr_fpcr(fpcr);
	msr_fpsr(fpsr);
	return (0);
}

int
feupdateenv(const fenv_t *envp)
{
	uint64_t fpsr;

	mrs_fpsr(fpsr);
	fesetenv(envp);
	feraiseexcept(fpsr & FE_ALL_EXCEPT);
	return (0);
}

int
feenableexcept(int mask)
{
	uint64_t fpcr, old;

	mrs_fpcr(fpcr);
	old = (fpcr >> ENABLE_SHIFT) & FE_ALL_EXCEPT;
	fpcr |= (uint64_t)(mask & FE_ALL_EXCEPT) << ENABLE_SHIFT;
	msr_fpcr(fpcr);
	return ((int)old);
}

int
fedisableexcept(int mask)
{
	uint64_t fpcr, old;

	mrs_fpcr(fpcr);
	old = (fpcr >> ENABLE_SHIFT) & FE_ALL_EXCEPT;
	fpcr &= ~((uint64_t)(mask & FE_ALL_EXCEPT) << ENABLE_SHIFT);
	msr_fpcr(fpcr);
	return ((int)old);
}

int
fegetexcept(void)
{
	uint64_t fpcr;

	mrs_fpcr(fpcr);
	return ((int)((fpcr >> ENABLE_SHIFT) & FE_ALL_EXCEPT));
}
