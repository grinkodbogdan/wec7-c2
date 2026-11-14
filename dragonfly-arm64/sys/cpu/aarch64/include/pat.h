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

#ifndef _CPU_PAT_H_
#define _CPU_PAT_H_

/*
 * Memory attribute types, as used for vm_memattr_t (<vm/vm.h>).
 *
 * The names and values are x86's Page Attribute Table encodings because
 * that is the vocabulary machine-independent code and drivers use.  On
 * aarch64 they are abstract: the pmap translates each one to a MAIR_EL1
 * attribute index (<cpu/pmap.h>) when it builds a PTE.
 *
 *	PAT_WRITE_BACK		normal cacheable memory
 *	PAT_WRITE_COMBINING	normal non-cacheable memory
 *	PAT_UNCACHEABLE		device memory (nGnRE)
 */
#define	PAT_UNCACHEABLE		0x00
#define	PAT_WRITE_COMBINING	0x01
#define	PAT_WRITE_THROUGH	0x04
#define	PAT_WRITE_PROTECTED	0x05
#define	PAT_WRITE_BACK		0x06
#define	PAT_UNCACHED		0x07

#endif /* !_CPU_PAT_H_ */
