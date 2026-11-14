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
 * Setup the system to run on the current machine.
 *
 * configure() is called at boot time and initializes the device
 * tables.  There is no device-tree bus (nexus/simplebus) yet, so the
 * root bus has no children and nothing is probed: this stage only
 * finishes interrupt setup and opens the console.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/bus.h>
#include <sys/conf.h>
#include <sys/cons.h>
#include <sys/machintr.h>
#include <sys/reboot.h>

#include <sys/thread2.h>

#include <machine/cpufunc.h>
#include <machine/md_var.h>

static void	configure_first(void *);
static void	configure(void *);
static void	configure_final(void *);

SYSINIT(configure1, SI_SUB_CONFIGURE, SI_ORDER_FIRST, configure_first, NULL);
/* SI_ORDER_SECOND is hookable */
SYSINIT(configure2, SI_SUB_CONFIGURE, SI_ORDER_THIRD, configure, NULL);
/* SI_ORDER_MIDDLE is hookable */
SYSINIT(configure3, SI_SUB_CONFIGURE, SI_ORDER_ANY, configure_final, NULL);

cdev_t	rootdev = NULL;
cdev_t	dumpdev = NULL;

static void
configure_first(void *dummy __unused)
{
}

/*
 * Determine i/o configuration for a machine.
 */
static void
configure(void *dummy __unused)
{
	/*
	 * Final interrupt support activation, then enable hardware
	 * interrupts.  From here on the clock runs.
	 */
	MachIntrABI.finalize();
	cpu_enable_intr();

	root_bus_configure();

	/*
	 * Allow lowering of the ipl to the lowest kernel level if we
	 * panic (or call tsleep() before clearing `cold').  No level is
	 * completely safe (since a panic may occur in a critical region
	 * at splhigh()), but we want at least bio interrupts to work.
	 */
	safepri = TDPRI_KERN_USER;
}

static void
configure_final(void *dummy __unused)
{
	cninit_finish();

	if (bootverbose)
		kprintf("Device configuration finished.\n");
}

/*
 * Pick the root filesystem.  Without storage drivers the only thing
 * that can be honoured is an explicit request from the boot arguments
 * (vfs.root.mountfrom), which the generic code reads on its own.
 */
void
cpu_rootconf(void)
{
}
SYSINIT(cpu_rootconf, SI_SUB_ROOT_CONF, SI_ORDER_FIRST, cpu_rootconf, NULL);
