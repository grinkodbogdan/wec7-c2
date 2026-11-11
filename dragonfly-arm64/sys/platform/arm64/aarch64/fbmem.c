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
 * /dev/fb0: the framebuffer the console draws on (the one LK left
 * scanning out on the Mi 5, or fb= on the command line), for userland
 * graphics until there is a display driver.  One client at a time;
 * while it has the device open the text console stops drawing, and
 * FBMEM_FLUSH pushes its drawing to the panel (the Mi 5's is a
 * command-mode panel that only shows what it is sent).
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/conf.h>
#include <sys/device.h>

#include <machine/fbmem.h>
#include <machine/md_var.h>

static d_open_t		fbmem_open;
static d_close_t	fbmem_close;
static d_ioctl_t	fbmem_ioctl;
static d_mmap_t		fbmem_mmap;

static struct dev_ops fbmem_ops = {
	{ "fbmem", 0, 0 },
	.d_open =	fbmem_open,
	.d_close =	fbmem_close,
	.d_ioctl =	fbmem_ioctl,
	.d_mmap =	fbmem_mmap,
};

static int fbmem_busy;

static int
fbmem_open(struct dev_open_args *ap __unused)
{
	if (atomic_cmpset_int(&fbmem_busy, 0, 1) == 0)
		return (EBUSY);
	fbcons_suspend(1);
	return (0);
}

static int
fbmem_close(struct dev_close_args *ap __unused)
{
	fbcons_suspend(0);
	fbmem_busy = 0;
	return (0);
}

static int
fbmem_ioctl(struct dev_ioctl_args *ap)
{
	struct fbmem_info *fi;
	vm_paddr_t pa;
	u_int w, h, stride, bpp;

	switch (ap->a_cmd) {
	case FBMEM_GETINFO:
		if (fbcons_geometry(&pa, &w, &h, &stride, &bpp) != 0)
			return (ENXIO);
		fi = (struct fbmem_info *)ap->a_data;
		fi->width = w;
		fi->height = h;
		fi->stride = stride;
		fi->bpp = bpp;
		fi->size = round_page((uint64_t)stride * h);
		return (0);
	case FBMEM_FLUSH:
		fbcons_push();
		return (0);
	case FBMEM_FLUSH_ROWS: {
		struct fbmem_rows *r = (struct fbmem_rows *)ap->a_data;

		fbcons_push_rows(r->y0, r->y1);
		return (0);
	}
	}
	return (ENOTTY);
}

static int
fbmem_mmap(struct dev_mmap_args *ap)
{
	vm_paddr_t pa;
	u_int w, h, stride, bpp;

	if (fbcons_geometry(&pa, &w, &h, &stride, &bpp) != 0)
		return (ENXIO);
	if (ap->a_offset < 0 ||
	    ap->a_offset >= (vm_ooffset_t)round_page((uint64_t)stride * h))
		return (EINVAL);
	ap->a_result = atop(pa + ap->a_offset);
	return (0);
}

static void
fbmem_attach(void *dummy __unused)
{
	vm_paddr_t pa;
	u_int w, h, stride, bpp;

	if (fbcons_geometry(&pa, &w, &h, &stride, &bpp) != 0)
		return;
	make_dev(&fbmem_ops, 0, UID_ROOT, GID_WHEEL, 0600, "fb0");
	kprintf("fb0: %ux%u, %u bits per pixel, at 0x%jx\n", w, h, bpp,
	    (uintmax_t)pa);
}
SYSINIT(fbmem, SI_SUB_DRIVERS, SI_ORDER_ANY, fbmem_attach, NULL);
