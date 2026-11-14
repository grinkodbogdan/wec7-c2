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
 * asm_ones32(), which netinet/in_cksum.c expects from the machine: the
 * 32-bit one's complement partial sum of count aligned 32-bit words,
 * without the final inversion (x86_64 has it in assembly, in_cksum2.s).
 */

#include <sys/types.h>
#include <sys/in_cksum.h>

__uint32_t
asm_ones32(const void *buf, int count)
{
	const __uint32_t *p = buf;
	__uint64_t sum = 0;

	while (count-- > 0)
		sum += *p++;
	sum = (sum & 0xffffffffU) + (sum >> 32);
	sum = (sum & 0xffffffffU) + (sum >> 32);
	return ((__uint32_t)sum);
}
