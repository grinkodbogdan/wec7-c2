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

#ifndef _MACHINE_PTRACE_H_
#define _MACHINE_PTRACE_H_

/*
 * Machine dependent trace commands.
 */
#define PT_GETREGS      (PT_FIRSTMACH + 1)
#define PT_SETREGS      (PT_FIRSTMACH + 2)
#define PT_GETFPREGS    (PT_FIRSTMACH + 3)
#define PT_SETFPREGS    (PT_FIRSTMACH + 4)
#define PT_GETDBREGS    (PT_FIRSTMACH + 5)
#define PT_SETDBREGS    (PT_FIRSTMACH + 6)

#endif
