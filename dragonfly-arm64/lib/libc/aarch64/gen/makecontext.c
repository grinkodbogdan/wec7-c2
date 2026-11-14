/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * After the x86_64 version: the new context starts in a wrapper that
 * calls the start function and then switches to uc_link (or exits).
 */
#include <sys/param.h>
#include <sys/signal.h>
#include <sys/ucontext.h>

#include <signal.h>
#include <stdarg.h>
#include <stdlib.h>
#include <ucontext.h>
#include <unistd.h>

void _makecontext(ucontext_t *, void (*)(void), int, ...);

typedef void (*func_t)(uint64_t, uint64_t, uint64_t, uint64_t,
		       uint64_t, uint64_t, uint64_t, uint64_t);

static void makectx_wrapper(ucontext_t *ucp, func_t func, uint64_t *args);

#define	NREGARGS	8	/* arguments passed in x0-x7 */

void
_makecontext(ucontext_t *ucp, void (*start)(void), int argc, ...)
{
	va_list ap;
	uint64_t *stack_top;
	uint64_t *argp;
	int i;

	if (ucp == NULL)
		return;
	if (ucp->uc_stack.ss_sp == NULL ||
	    ucp->uc_stack.ss_size < MINSIGSTKSZ)
		ucp->uc_mcontext.mc_len = 0;
	if (argc < 0 || argc > NREGARGS)
		ucp->uc_mcontext.mc_len = 0;
	if (ucp->uc_mcontext.mc_len != sizeof(mcontext_t))
		return;

	/* The arguments sit at the top of the new stack. */
	stack_top = (uint64_t *)((char *)ucp->uc_stack.ss_sp +
	    ucp->uc_stack.ss_size);
	stack_top = (uint64_t *)((uintptr_t)stack_top & ~15UL);
	argp = stack_top - NREGARGS;
	stack_top = argp;		/* 64 bytes: still 16-aligned */

	va_start(ap, argc);
	for (i = 0; i < argc; i++)
		argp[i] = va_arg(ap, uint64_t);
	va_end(ap);
	for (; i < NREGARGS; i++)
		argp[i] = 0;

	ucp->uc_mcontext.mc_x[0] = (register_t)ucp;
	ucp->uc_mcontext.mc_x[1] = (register_t)start;
	ucp->uc_mcontext.mc_x[2] = (register_t)argp;
	ucp->uc_mcontext.mc_x[29] = 0;		/* end of frame chain */
	ucp->uc_mcontext.mc_lr = 0;
	ucp->uc_mcontext.mc_sp = (register_t)stack_top;
	ucp->uc_mcontext.mc_elr = (register_t)makectx_wrapper;
	ucp->uc_mcontext.mc_spsr = 0;		/* EL0t */
	ucp->uc_mcontext.mc_onstack = 0;
}

__weak_reference(_makecontext, makecontext);

static void
makectx_wrapper(ucontext_t *ucp, func_t func, uint64_t *args)
{
	(*func)(args[0], args[1], args[2], args[3], args[4], args[5],
	    args[6], args[7]);
	if (ucp->uc_link == NULL)
		exit(0);
	setcontext((const ucontext_t *)ucp->uc_link);
	abort();
}
