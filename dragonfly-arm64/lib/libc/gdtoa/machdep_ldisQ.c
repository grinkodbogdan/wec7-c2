/*-
 * Copyright (c) 2003 David Schultz <das@FreeBSD.ORG>
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 * 1. Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in the
 *    documentation and/or other materials provided with the distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
 * ANY EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.
 *
 * Machine-dependent glue to integrate David Gay's gdtoa package into
 * libc for architectures where a long double uses quad precision, such
 * as aarch64.  From FreeBSD.
 */

#include <float.h>

#include "gdtoaimp.h"

long double
strtold_l(const char * __restrict s, char ** __restrict sp, locale_t locale)
{
	long double result;
	FIX_LOCALE(locale);

	strtorQ_l(s, sp, FLT_ROUNDS, &result, locale);
	return result;
}
