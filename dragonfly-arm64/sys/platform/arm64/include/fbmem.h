/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * /dev/fb0: the boot framebuffer for userland (see fbmem.c).
 */
#ifndef _MACHINE_FBMEM_H_
#define	_MACHINE_FBMEM_H_

#include <sys/types.h>
#include <sys/ioccom.h>

struct fbmem_info {
	uint32_t	width;		/* pixels */
	uint32_t	height;
	uint32_t	stride;		/* bytes per line */
	uint32_t	bpp;		/* 16, 24 or 32 */
	uint64_t	size;		/* bytes to mmap */
};

/* Geometry. */
#define	FBMEM_GETINFO	_IOR('F', 1, struct fbmem_info)
/* Make what was drawn visible (cache clean, panel refresh). */
#define	FBMEM_FLUSH	_IO('F', 2)

/* The same for rows [y0, y1) only: less cache to clean. */
struct fbmem_rows {
	uint32_t	y0;
	uint32_t	y1;
};
#define	FBMEM_FLUSH_ROWS _IOW('F', 3, struct fbmem_rows)

#endif /* !_MACHINE_FBMEM_H_ */
