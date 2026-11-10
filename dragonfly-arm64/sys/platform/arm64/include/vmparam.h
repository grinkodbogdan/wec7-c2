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

#ifndef _MACHINE_VMPARAM_H_
#define _MACHINE_VMPARAM_H_

/*
 * Machine dependent constants for the arm64 platform.
 *
 * The kernel half of the address space deliberately uses the same
 * layout as pc64 so machine-independent assumptions about where
 * things live keep holding:
 *
 *	0x0000000000000000 - 0x00007fffffffffff	userland (TTBR0)
 *	0xffff800000000000 - 0xffffffffffffffff	kernel   (TTBR1)
 *	    0xfffff80000000000	DMAP, direct map of physical memory
 *	    0xffffffff80000000	KERNBASE, kernel text/data
 */

/*
 * Virtual memory related constants, all in bytes
 */
#define	MAXTSIZ		(32UL*1024*1024*1024)		/* max text size */
#ifndef DFLDSIZ
#define	DFLDSIZ		(128UL*1024*1024)		/* initial data size limit */
#endif
#ifndef MAXDSIZ
#define	MAXDSIZ		(32UL*1024*1024*1024)		/* max data size */
#endif
#ifndef	DFLSSIZ
#define	DFLSSIZ		(8UL*1024*1024)			/* initial stack size limit */
#endif
#ifndef	MAXSSIZ
#define	MAXSSIZ		(512UL*1024*1024)		/* max stack size */
#endif
#ifndef	MAXTHRSSIZ
#define	MAXTHRSSIZ	(128UL*1024*1024*1024)		/* thread stack area */
#endif
#ifndef SGROWSIZ
#define	SGROWSIZ	(128UL*1024)			/* amount to grow stack */
#endif

/*
 * The time for a process to be blocked before being very swappable.
 */
#define	MAXSLP		20

#define	KERNBASE		0xffffffff80000000UL

#define	DMAP_MIN_ADDRESS	0xfffff80000000000UL
#define	DMAP_MAX_ADDRESS	0xfffffc0000000000UL	/* 4TB of DMAP */

#define	VM_MIN_USER_ADDRESS	0x0000000000000000UL
#define	VM_MAX_USER_ADDRESS	0x0000800000000000UL

#define	USRSTACK		VM_MAX_USER_ADDRESS

#define	VM_MIN_KERNEL_ADDRESS	0xffff800000000000UL
#define	VM_MAX_KERNEL_ADDRESS	0xffffffffffffffffUL

#define	VM_MAX_ADDRESS		VM_MAX_USER_ADDRESS
#define	VM_MIN_ADDRESS		VM_MIN_USER_ADDRESS

#define	UPT_MAX_ADDRESS		VM_MAX_USER_ADDRESS
#define	UPT_MIN_ADDRESS		VM_MIN_USER_ADDRESS

#define	PHYS_TO_DMAP(x)		((vm_offset_t)(x) | DMAP_MIN_ADDRESS)
#define	DMAP_TO_PHYS(x)		((vm_paddr_t)(x) & ~DMAP_MIN_ADDRESS)

/* initial pagein size of beginning of executable file */
#ifndef VM_INITIAL_PAGEIN
#define	VM_INITIAL_PAGEIN	16
#endif

/* number of contiguous physical memory segments tracked at boot */
#define	VM_PHYSSEG_MAX		32

#endif /* _MACHINE_VMPARAM_H_ */
