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

#ifndef _CPU_VFRAME_H_
#define _CPU_VFRAME_H_

#ifndef _SYS_TLS_H_
#include <sys/tls.h>
#endif

/*
 * Per-thread TLS descriptors saved across context switches.  Slot 0 is
 * the userland thread pointer (TPIDR_EL0).
 */
struct savetls {
	struct tls_info info[2];
};

/*
 * Virtualized external frame.  This is used by the virtual kernel in
 * addition to trapframe.
 */
struct vextframe {
	struct savetls vx_tls;
};

#endif
