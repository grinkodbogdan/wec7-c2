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
 * Minimal flattened device tree scanner used during early boot, before
 * any allocator or bus framework exists.  It makes a single pass over
 * the structure block and records the handful of facts the kernel needs
 * to reach the console, the interrupt controller and physical memory.
 *
 * Limitations (all acceptable for bring-up, none for the final port):
 *   - "reg" is decoded using the root node's #address-cells/#size-cells,
 *     so devices must sit directly under the root or under a bus with
 *     the same cell counts and an identity "ranges".
 *   - only the first matching UART and interrupt controller are kept.
 */

#include <sys/types.h>
#include <sys/_null.h>
#include <machine/md_var.h>

void	bzero(void *, size_t);

#define	FDT_MAGIC	0xd00dfeed
#define	FDT_BEGIN_NODE	1
#define	FDT_END_NODE	2
#define	FDT_PROP	3
#define	FDT_NOP		4
#define	FDT_END		9

#define	FDT_MAXDEPTH	16

struct fdt_header {
	uint32_t	magic;
	uint32_t	totalsize;
	uint32_t	off_dt_struct;
	uint32_t	off_dt_strings;
	uint32_t	off_mem_rsvmap;
	uint32_t	version;
	uint32_t	last_comp_version;
	uint32_t	boot_cpuid_phys;
	uint32_t	size_dt_strings;
	uint32_t	size_dt_struct;
};

/* Properties of interest collected for the node being walked. */
struct fdt_node {
	const char	*name;
	const char	*compat;
	uint32_t	compat_len;
	const uint32_t	*reg;
	uint32_t	reg_len;
	const uint32_t	*intr;
	uint32_t	intr_len;
	const char	*device_type;
	const char	*method;
	int		done;
};

static uint32_t
fdt32(const uint32_t *p)
{
	return (__builtin_bswap32(*p));
}

static int
streq(const char *a, const char *b)
{
	while (*a != '\0' && *a == *b) {
		++a;
		++b;
	}
	return (*a == *b);
}

static int
strprefix(const char *s, const char *prefix)
{
	while (*prefix != '\0') {
		if (*s++ != *prefix++)
			return (0);
	}
	return (1);
}

static size_t
slen(const char *s)
{
	size_t n = 0;

	while (s[n] != '\0')
		++n;
	return (n);
}

/*
 * "compatible" is a list of NUL terminated strings.
 */
static int
fdt_is_compatible(const struct fdt_node *n, const char *what)
{
	const char *p = n->compat;
	const char *end = p + n->compat_len;

	while (p != NULL && p < end) {
		if (streq(p, what))
			return (1);
		p += slen(p) + 1;
	}
	return (0);
}

static uint64_t
fdt_cells(const uint32_t *p, int ncells)
{
	uint64_t v = 0;

	while (ncells-- > 0)
		v = (v << 32) | fdt32(p++);
	return (v);
}

/*
 * Called once per node, after all of its properties have been seen.
 */
static void
fdt_node_done(struct fdt_node *n, int depth, int acells, int scells,
	      struct arm64_bootinfo *bi)
{
	int cells = acells + scells;
	uint32_t off;

	if (n->done)
		return;
	n->done = 1;

	if (n->device_type != NULL && streq(n->device_type, "memory") &&
	    n->reg != NULL) {
		for (off = 0; off + cells <= n->reg_len / 4; off += cells) {
			if (bi->bi_nmem == ARM64_MAXMEMREGIONS)
				break;
			bi->bi_mem[bi->bi_nmem].mr_start =
			    fdt_cells(n->reg + off, acells);
			bi->bi_mem[bi->bi_nmem].mr_size =
			    fdt_cells(n->reg + off + acells, scells);
			++bi->bi_nmem;
		}
	}
	if (n->device_type != NULL && streq(n->device_type, "cpu"))
		++bi->bi_ncpus;

	if (n->compat == NULL)
		return;

	if (bi->bi_uart_base == 0 && n->reg != NULL &&
	    fdt_is_compatible(n, "arm,pl011"))
		bi->bi_uart_base = fdt_cells(n->reg, acells);

	if (bi->bi_gic_version == 0 && n->reg != NULL) {
		if (fdt_is_compatible(n, "arm,cortex-a15-gic") ||
		    fdt_is_compatible(n, "arm,gic-400")) {
			bi->bi_gic_version = 2;
			bi->bi_gicd_base = fdt_cells(n->reg, acells);
			if (n->reg_len / 4 >= (uint32_t)(2 * cells)) {
				bi->bi_gicc_base =
				    fdt_cells(n->reg + cells, acells);
			}
		} else if (fdt_is_compatible(n, "arm,gic-v3")) {
			bi->bi_gic_version = 3;
			bi->bi_gicd_base = fdt_cells(n->reg, acells);
		}
	}

	/*
	 * interrupts = <sec-phys> <nonsec-phys> <virt> <hyp>, each a
	 * (type, number, flags) triplet.  type 1 is a PPI, whose INTID
	 * is 16 + number.
	 */
	if (fdt_is_compatible(n, "arm,armv8-timer") && n->intr != NULL &&
	    n->intr_len >= 9 * 4) {
		bi->bi_timer_irq = fdt32(n->intr + 7) +
		    (fdt32(n->intr + 6) == 1 ? 16 : 32);
	}

	if (n->method != NULL && (fdt_is_compatible(n, "arm,psci-0.2") ||
	    fdt_is_compatible(n, "arm,psci-1.0") ||
	    fdt_is_compatible(n, "arm,psci"))) {
		bi->bi_psci = streq(n->method, "smc") ? 2 : 1;
	}
}

/*
 * Walk the device tree at (fdt) and fill in (bi).  Returns 0 on
 * success or -1 if the blob does not look like a device tree.
 */
int
fdt_scan(const void *fdt, struct arm64_bootinfo *bi)
{
	const struct fdt_header *hdr = fdt;
	const uint32_t *p;
	const uint32_t *end;
	const char *strings;
	struct fdt_node nodes[FDT_MAXDEPTH];
	struct fdt_node *n;
	int depth = -1;
	int acells = 2;
	int scells = 1;
	int in_chosen = 0;

	if (fdt32(&hdr->magic) != FDT_MAGIC)
		return (-1);
	bi->bi_fdt_size = fdt32(&hdr->totalsize);
	p = (const uint32_t *)((const char *)fdt + fdt32(&hdr->off_dt_struct));
	end = (const uint32_t *)((const char *)p + fdt32(&hdr->size_dt_struct));
	strings = (const char *)fdt + fdt32(&hdr->off_dt_strings);

	while (p < end) {
		uint32_t token = fdt32(p++);

		switch (token) {
		case FDT_BEGIN_NODE: {
			const char *name = (const char *)p;

			if (depth >= 0 && depth < FDT_MAXDEPTH) {
				fdt_node_done(&nodes[depth], depth, acells,
					      scells, bi);
			}
			++depth;
			if (depth < FDT_MAXDEPTH) {
				n = &nodes[depth];
				bzero(n, sizeof(*n));
				n->name = name;
			}
			if (depth == 1)
				in_chosen = streq(name, "chosen");
			p += (slen(name) + 1 + 3) / 4;
			break;
		}
		case FDT_END_NODE:
			if (depth >= 0 && depth < FDT_MAXDEPTH) {
				fdt_node_done(&nodes[depth], depth, acells,
					      scells, bi);
			}
			if (depth == 1)
				in_chosen = 0;
			--depth;
			break;
		case FDT_PROP: {
			uint32_t len = fdt32(p++);
			const char *pname = strings + fdt32(p++);
			const void *data = p;

			p += (len + 3) / 4;
			if (depth < 0 || depth >= FDT_MAXDEPTH)
				break;
			n = &nodes[depth];
			if (streq(pname, "compatible")) {
				n->compat = data;
				n->compat_len = len;
				if (depth == 0)
					bi->bi_compatible = data;
			} else if (streq(pname, "reg")) {
				n->reg = data;
				n->reg_len = len;
			} else if (streq(pname, "interrupts")) {
				n->intr = data;
				n->intr_len = len;
			} else if (streq(pname, "device_type")) {
				n->device_type = data;
			} else if (streq(pname, "method")) {
				n->method = data;
			} else if (depth == 0 && streq(pname, "model")) {
				bi->bi_model = data;
			} else if (depth == 0 &&
				   streq(pname, "#address-cells")) {
				acells = fdt32(data);
			} else if (depth == 0 && streq(pname, "#size-cells")) {
				scells = fdt32(data);
			} else if (in_chosen && depth == 1 &&
				   streq(pname, "bootargs")) {
				bi->bi_bootargs = data;
			} else if (in_chosen && depth == 1 &&
				   streq(pname, "stdout-path")) {
				bi->bi_stdout = data;
			}
			break;
		}
		case FDT_NOP:
			break;
		case FDT_END:
			return (0);
		default:
			return (-1);
		}
	}
	(void)strprefix;
	return (0);
}
