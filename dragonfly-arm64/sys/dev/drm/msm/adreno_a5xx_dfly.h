/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Adreno 5xx bring-up for DragonFly/arm64; see adreno_a5xx_dfly.c.
 */
#ifndef _ADRENO_A5XX_DFLY_H_
#define	_ADRENO_A5XX_DFLY_H_

typedef void (*a5xx_status_fn)(const char *fmt, ...) __printflike(1, 2);

/*
 * Start the powered GPU (whose zap shader is loaded) with the PM4 and
 * PFP microcode files as read from disk; 0 when a CP_MEM_WRITE came back.
 */
int	a5xx_dfly_start(a5xx_status_fn status, const uint8_t *pm4,
	    size_t pm4_len, const uint8_t *pfp, size_t pfp_len);

#endif
