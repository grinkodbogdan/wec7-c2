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

#ifndef _MACHINE_METADATA_H_
#define	_MACHINE_METADATA_H_

/* Flattened device tree handed over by the loader. */
#define	MODINFOMD_DTBP		0x1001
#define	MODINFOMD_EFI_MAP	0x1004
#define	MODINFOMD_EFI_FB	0x1005

struct efi_map_header {
	uint64_t	memory_size;
	uint64_t	descriptor_size;
	uint32_t	descriptor_version;
};

struct efi_fb {
	uint64_t	fb_addr;
	uint64_t	fb_size;
	uint32_t	fb_height;
	uint32_t	fb_width;
	uint32_t	fb_stride;
	uint32_t	fb_mask_red;
	uint32_t	fb_mask_green;
	uint32_t	fb_mask_blue;
	uint32_t	fb_mask_reserved;
};

#endif /* !_MACHINE_METADATA_H_ */
