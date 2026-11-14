/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Adreno 5xx bring-up for DragonFly/arm64; see adreno_a5xx_dfly.c.
 */
#ifndef _ADRENO_A5XX_DFLY_H_
#define	_ADRENO_A5XX_DFLY_H_

typedef void (*a5xx_status_fn)(const char *fmt, ...) __printflike(1, 2);

/*
 * Start the powered GPU with the PM4 and PFP microcode files as read
 * from disk; 0 when a CP_MEM_WRITE came back.  zap loads the zap shader
 * through the secure world: as in Linux's a5xx_hw_init it is called once
 * the CP has run CP_ME_INIT, before CP_SET_SECURE_MODE.
 */
int	a5xx_dfly_start(a5xx_status_fn status, int (*zap)(void),
	    const uint8_t *pm4, size_t pm4_len, const uint8_t *pfp,
	    size_t pfp_len);

/* After a5xx_dfly_start: */
int	a5xx_dfly_running(void);
int	a5xx_dfly_submit(const uint64_t *iova, const uint32_t *ndw, int n,
	    uint32_t seqno);
uint32_t a5xx_dfly_fence(void);
uint64_t a5xx_dfly_timestamp(void);

/* msm_dfly.c: create /dev/dri/card0 once the GPU runs. */
void	msm_dfly_attach(void);

#endif
