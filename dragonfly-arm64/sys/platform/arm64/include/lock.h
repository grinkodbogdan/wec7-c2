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

#ifndef _MACHINE_LOCK_H_
#define _MACHINE_LOCK_H_

#ifndef LOCORE
#ifdef _KERNEL

/*
 * Deprecated spinlocks which disable interrupts while held; kept for
 * the few machine-dependent users (clock, console).
 */
struct spinlock_deprecated {
	volatile long	opaque;
};

void	com_lock(void);		/* disables int / spinlock combo */
void	com_unlock(void);
void	clock_lock(void);	/* disables int / spinlock combo */
void	clock_unlock(void);

void	spin_lock_deprecated(struct spinlock_deprecated *lock);
void	spin_unlock_deprecated(struct spinlock_deprecated *lock);

static __inline void
spin_init_deprecated(struct spinlock_deprecated *lock)
{
	lock->opaque = 0;
}

#endif  /* _KERNEL */
#endif	/* LOCORE */
#endif	/* !_MACHINE_LOCK_H_ */
