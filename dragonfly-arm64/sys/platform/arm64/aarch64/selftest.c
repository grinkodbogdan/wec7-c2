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
 * Boot-time checks of the primitives in <cpu/atomic.h> and
 * <cpu/cpufunc.h>.  Machine-independent code depends on the exact
 * return value conventions of these, and they are easy to get subtly
 * wrong.  Run by putting "selftest" in the boot arguments.
 */

#include <sys/param.h>
#include <sys/systm.h>

#include <machine/atomic.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>

static int selftest_failures;

#define	CHECK(expr)							\
do {									\
	if (!(expr)) {							\
		kprintf("selftest FAILED: %s\n", #expr);		\
		++selftest_failures;					\
	}								\
} while (0)

static void
selftest_nop(void *arg __unused)
{
}

void
arm64_selftest(void)
{
	volatile u_int i = 5;
	volatile u_long l = 0x100000000UL;
	u_int iold;
	u_long lold;
	__atomic_intr_t ai;

	atomic_add_int(&i, 3);
	CHECK(i == 8);
	atomic_subtract_int(&i, 1);
	CHECK(i == 7);
	atomic_set_int(&i, 0x10);
	CHECK(i == 0x17);
	atomic_clear_int(&i, 0x3);
	CHECK(i == 0x14);
	CHECK(atomic_fetchadd_int(&i, 4) == 0x14 && i == 0x18);
	CHECK(atomic_cmpset_int(&i, 0x18, 1) != 0 && i == 1);
	CHECK(atomic_cmpset_int(&i, 0x18, 2) == 0 && i == 1);
	iold = 9;
	CHECK(atomic_fcmpset_int(&i, &iold, 2) == 0 && iold == 1);
	CHECK(atomic_fcmpset_int(&i, &iold, 2) != 0 && i == 2);
	CHECK(atomic_swap_int((volatile int *)&i, 40) == 2 && i == 40);
	CHECK(atomic_readandclear_int(&i) == 40 && i == 0);
	CHECK(atomic_testandset_int(&i, 3) == 0 && i == 8);
	CHECK(atomic_testandset_int(&i, 3) != 0);
	CHECK(atomic_testandclear_int(&i, 3) != 0 && i == 0);
	CHECK(atomic_poll_acquire_int(&i) != 0);
	CHECK(atomic_poll_acquire_int(&i) == 0);
	atomic_poll_release_int(&i);
	CHECK(i == 0);

	atomic_add_long(&l, 0x100000000UL);
	CHECK(l == 0x200000000UL);
	CHECK(atomic_cmpset_long(&l, 0x200000000UL, 0xffffffffffUL) != 0);
	lold = 0;
	CHECK(atomic_fcmpset_long(&l, &lold, 1) == 0 &&
	    lold == 0xffffffffffUL);
	CHECK(atomic_fetchadd_long(&l, 1) == 0xffffffffffUL &&
	    l == 0x10000000000UL);
	CHECK(atomic_testandset_long(&l, 63) == 0 &&
	    l == 0x8000010000000000UL);
	CHECK(atomic_load_acq_long(&l) == 0x8000010000000000UL);
	atomic_store_rel_long(&l, 0);
	CHECK(l == 0);

	atomic_intr_init(&ai);
	CHECK(atomic_intr_cond_try(&ai) == 0);
	CHECK(atomic_intr_cond_try(&ai) == 1);
	CHECK(atomic_intr_cond_test(&ai) != 0);
	CHECK(atomic_intr_handler_disable(&ai) != 0);
	CHECK(atomic_intr_handler_is_enabled(&ai) != 0);
	atomic_intr_handler_enable(&ai);
	CHECK(atomic_intr_handler_is_enabled(&ai) == 0);
	atomic_intr_cond_exit(&ai, selftest_nop, NULL);
	CHECK(ai == 0);

	CHECK(bsfl(0x80) == 7 && bsrl(0x80) == 7);
	CHECK(bsfq(0x8000000000UL) == 39 && bsrq(0x8000000001UL) == 39);
	CHECK(ffs(0) == 0 && ffs(0x18) == 4);
	CHECK(fls(0) == 0 && fls(0x18) == 5 && fls(-1) == 32);
	CHECK(ffsl(0x100000000L) == 33 && flsl(0x100000000L) == 33);
	CHECK(__bswap32(0x11223344) == 0x44332211);
	CHECK(__bswap16(0x1122) == 0x2211);
	CHECK(__bswap64(0x1122334455667788UL) == 0x8877665544332211UL);

	kprintf("selftest: atomic and bit primitives %s\n",
	    selftest_failures ? "FAILED" : "passed");
}
