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
 * Qualcomm secure monitor calls (SCM), the ARMv8 SMC calling convention
 * as Qualcomm's TrustZone uses it.  The protocol follows Linux's
 * drivers/firmware/qcom/qcom_scm-smc.c: function id
 * 0x42000000 | service << 8 | command (SMC64, owner SiP; bit 31 set for
 * a fast call), x1 the argument description (count, then two bits of
 * type per argument), x2-x5 the arguments; x0 returns the status, x1-x3
 * the results.  A return of 1 (interrupted) is resumed by calling again
 * with that value in x0.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>

#include <machine/md_var.h>

#define	SMCCC_FAST		(1UL << 31)
#define	SMCCC_SMC64		(1UL << 30)
#define	SMCCC_OWNER_SIP		(2UL << 24)
#define	SCM_INTERRUPTED		1
#define	SCM_V2_EBUSY		(-12)

static struct lock scm_lock = LOCK_INITIALIZER("qcomscm", 0, 0);

static void
smc(uint64_t a[8], uint64_t r[7])
{
	register uint64_t x0 __asm("x0") = a[0];
	register uint64_t x1 __asm("x1") = a[1];
	register uint64_t x2 __asm("x2") = a[2];
	register uint64_t x3 __asm("x3") = a[3];
	register uint64_t x4 __asm("x4") = a[4];
	register uint64_t x5 __asm("x5") = a[5];
	register uint64_t x6 __asm("x6") = a[6];
	register uint64_t x7 __asm("x7") = a[7];

	__asm __volatile("smc #0"
	    : "+r" (x0), "+r" (x1), "+r" (x2), "+r" (x3),
	      "+r" (x4), "+r" (x5), "+r" (x6), "+r" (x7)
	    :
	    : "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15",
	      "x16", "x17", "memory");
	r[0] = x0;
	r[1] = x1;
	r[2] = x2;
	r[3] = x3;
	r[6] = x6;
}

/*
 * One call: service, command, argument description (QCOM_SCM_ARGS),
 * up to four arguments; res (optional) gets x1-x3.  Returns the
 * secure world's status (0 on success, negative on error).
 */
int
qcom_scm_call(int svc, int cmd, uint32_t arginfo, const uint64_t *args,
    int nargs, uint64_t *res, int fast)
{
	uint64_t a[8], r[7];
	int i, tries;

	lockmgr(&scm_lock, LK_EXCLUSIVE);
	for (tries = 0; tries < 20; ++tries) {
		memset(a, 0, sizeof(a));
		a[0] = (fast ? SMCCC_FAST : 0) | SMCCC_SMC64 | SMCCC_OWNER_SIP |
		    ((svc & 0xff) << 8) | (cmd & 0xff);
		a[1] = arginfo;
		for (i = 0; i < nargs && i < 4; ++i)
			a[2 + i] = args[i];
		/*
		 * An interrupted call is resumed with x0 = 1 and, as Linux's
		 * ARM_SMCCC_QUIRK_QCOM_A6 does, the x6 the secure world
		 * returned (its context for the call); the other arguments
		 * stay as they were.  Between attempts the cpu takes the
		 * interrupt that stopped it and may run other threads.
		 */
		a[6] = 0;
		for (;;) {
			smc(a, r);
			if (r[0] != SCM_INTERRUPTED)
				break;
			a[0] = r[0];
			a[6] = r[6];
			lwkt_yield();
		}
		if ((int64_t)r[0] != SCM_V2_EBUSY)
			break;
		tsleep(&scm_lock, 0, "scmbusy", hz * 30 / 1000 + 1);
	}
	lockmgr(&scm_lock, LK_RELEASE);
	if (res != NULL) {
		res[0] = r[1];
		res[1] = r[2];
		res[2] = r[3];
	}
	return ((int)(int64_t)r[0]);
}

/* QCOM_SCM_SVC_INFO / IS_CALL_AVAIL: 1 if (svc, cmd) exists, 0 if not. */
int
qcom_scm_is_available(int svc, int cmd)
{
	uint64_t arg = SMCCC_OWNER_SIP | ((svc & 0xff) << 8) | (cmd & 0xff);
	uint64_t res[3];
	int error;

	error = qcom_scm_call(0x06, 0x01, 1, &arg, 1, res, 0);
	if (error)
		return (error);
	return (res[0] != 0);
}
