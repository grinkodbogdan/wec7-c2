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

#ifndef _CPU_ENDIAN_H_
#define	_CPU_ENDIAN_H_

#include <sys/cdefs.h>
#include <machine/stdint.h>

/*
 * Define the order of 32-bit words in 64-bit words.
 */
#define	_QUAD_HIGHWORD 1
#define	_QUAD_LOWWORD 0

/*
 * Definitions for byte order, according to byte significance from low
 * address to high.  DragonFly/aarch64 is little-endian only.
 */
#define	_LITTLE_ENDIAN	1234	/* LSB first: i386, vax */
#define	_BIG_ENDIAN	4321	/* MSB first: 68000, ibm, net */
#define	_PDP_ENDIAN	3412	/* LSB first in word, MSW first in long */

#define	_BYTE_ORDER	_LITTLE_ENDIAN

#if __BSD_VISIBLE
#define	LITTLE_ENDIAN	_LITTLE_ENDIAN
#define	BIG_ENDIAN	_BIG_ENDIAN
#define	PDP_ENDIAN	_PDP_ENDIAN
#define	BYTE_ORDER	_BYTE_ORDER
#endif

#ifdef __GNUC__

/*
 * The compiler turns these into a single REV instruction.
 */
static __always_inline __uint64_t
__bswap64(__uint64_t _x)
{
	return (__builtin_bswap64(_x));
}

static __always_inline __uint32_t
__bswap32(__uint32_t _x)
{
	return (__builtin_bswap32(_x));
}

static __always_inline __uint16_t
__bswap16(__uint16_t _x)
{
	return (__builtin_bswap16(_x));
}

#define	__htonl(x)	__bswap32(x)
#define	__htons(x)	__bswap16(x)
#define	__ntohl(x)	__bswap32(x)
#define	__ntohs(x)	__bswap16(x)

#else /* !__GNUC__ */

#define	_BYTEORDER_FUNC_DEFINED

#endif /* __GNUC__ */

/*
 * Userland compatibility
 */
#ifndef _KERNEL
#define	__LITTLE_ENDIAN	_LITTLE_ENDIAN
#define	__BIG_ENDIAN	_BIG_ENDIAN
#define	__PDP_ENDIAN	_PDP_ENDIAN
#define	__BYTE_ORDER	_BYTE_ORDER
#ifndef __FLOAT_WORD_ORDER
#define	__FLOAT_WORD_ORDER __BYTE_ORDER
#endif
#endif /* !_KERNEL */

#endif /* !_CPU_ENDIAN_H_ */
