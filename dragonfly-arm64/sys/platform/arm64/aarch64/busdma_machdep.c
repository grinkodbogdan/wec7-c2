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
 * Bus DMA for arm64.  Two things set it apart from pc64's:
 *
 * - The MSM8996's PCIe is not cache-coherent.  Memory from
 *   bus_dmamem_alloc() is mapped normal non-cacheable (descriptor rings
 *   the device and the CPU share), and buffers loaded into a map are
 *   cleaned or invalidated from the cache in bus_dmamap_sync().
 * - Devices often reach only the low 4 GB (the QCA6174 does) while RAM
 *   goes above it.  A map whose buffer lies beyond the tag's lowaddr
 *   bounces through a buffer below it, copied in on PREWRITE and out on
 *   POSTREAD.
 *
 * There is no IOMMU here: a bus address is a physical address.  Each map
 * keeps the CPU addresses of what it loaded, so sync knows what to clean.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/malloc.h>
#include <sys/mbuf.h>
#include <sys/uio.h>
#include <sys/bus_dma.h>

#include <vm/vm.h>
#include <vm/pmap.h>
#include <vm/vm_extern.h>

#include <machine/pat.h>
#include <machine/md_var.h>

static MALLOC_DEFINE(M_BUSDMA, "busdma", "bus DMA tags and maps");

struct bus_dma_tag {
	bus_size_t	alignment;
	bus_size_t	boundary;
	bus_addr_t	lowaddr;
	bus_addr_t	highaddr;
	bus_size_t	maxsize;
	int		nsegments;
	bus_size_t	maxsegsz;
	int		flags;
};

/* one stretch of loaded memory, as the CPU sees it */
struct dma_piece {
	void		*kva;		/* what the device reads/writes */
	void		*orig;		/* the caller's buffer, if bounced */
	bus_size_t	len;
};

#define	MAX_PIECES	32

struct bus_dmamap {
	bus_dma_tag_t	tag;
	int		coherent;	/* from bus_dmamem_alloc */
	void		*mem;		/* bus_dmamem_alloc's memory */
	void		*mem_nc;	/* ... and its uncached mapping */
	vm_paddr_t	mem_pa;
	void		*bounce;	/* below the tag's lowaddr */
	vm_paddr_t	bounce_pa;
	bus_size_t	bounce_size;
	bus_size_t	bounce_used;
	int		npieces;
	struct dma_piece pieces[MAX_PIECES];
	int		nsegs;
	bus_dma_segment_t *segs;
};

static void
dcache(void *kva, bus_size_t len, int op)
{
	vm_offset_t va = rounddown2((vm_offset_t)kva, 64);
	vm_offset_t end = (vm_offset_t)kva + len;

	for (; va < end; va += 64) {
		if (op == 0)		/* clean: the device will read */
			__asm __volatile("dc cvac, %0" : : "r" (va) : "memory");
		else			/* clean + invalidate: it writes */
			__asm __volatile("dc civac, %0" : : "r" (va) : "memory");
	}
	__asm __volatile("dsb sy" : : : "memory");
}

int
bus_dma_tag_create(bus_dma_tag_t parent, bus_size_t alignment,
    bus_size_t boundary, bus_addr_t lowaddr, bus_addr_t highaddr,
    bus_size_t maxsize, int nsegments, bus_size_t maxsegsz, int flags,
    bus_dma_tag_t *dmat)
{
	struct bus_dma_tag *t;

	t = kmalloc(sizeof(*t), M_BUSDMA, M_INTWAIT | M_ZERO);
	t->alignment = alignment ? alignment : 1;
	t->boundary = boundary;
	t->lowaddr = lowaddr;
	t->highaddr = highaddr;
	t->maxsize = maxsize;
	t->nsegments = nsegments > 0 ? nsegments : 1;
	t->maxsegsz = maxsegsz ? maxsegsz : maxsize;
	t->flags = flags;
	if (parent != NULL) {
		t->lowaddr = MIN(t->lowaddr, parent->lowaddr);
		t->alignment = MAX(t->alignment, parent->alignment);
		if (parent->boundary != 0)
			t->boundary = t->boundary ? MIN(t->boundary,
			    parent->boundary) : parent->boundary;
	}
	*dmat = t;
	return (0);
}

int
bus_dma_tag_destroy(bus_dma_tag_t dmat)
{
	if (dmat != NULL)
		kfree(dmat, M_BUSDMA);
	return (0);
}

bus_size_t
bus_dma_tag_getmaxsize(bus_dma_tag_t tag)
{
	return (tag->maxsize);
}

int
bus_dmamap_create(bus_dma_tag_t dmat, int flags __unused, bus_dmamap_t *mapp)
{
	struct bus_dmamap *m;

	m = kmalloc(sizeof(*m), M_BUSDMA, M_INTWAIT | M_ZERO);
	m->tag = dmat;
	m->segs = kmalloc(sizeof(bus_dma_segment_t) * dmat->nsegments,
	    M_BUSDMA, M_INTWAIT | M_ZERO);
	*mapp = m;
	return (0);
}

int
bus_dmamap_destroy(bus_dma_tag_t dmat __unused, bus_dmamap_t map)
{
	if (map == NULL)
		return (0);
	if (map->bounce != NULL)
		contigfree(map->bounce, map->bounce_size, M_BUSDMA);
	kfree(map->segs, M_BUSDMA);
	kfree(map, M_BUSDMA);
	return (0);
}

int
bus_dmamem_alloc(bus_dma_tag_t dmat, void **vaddr, int flags,
    bus_dmamap_t *mapp)
{
	bus_dmamap_t map;
	bus_size_t size = round_page(dmat->maxsize);
	void *p;

	bus_dmamap_create(dmat, 0, &map);
	p = contigmalloc(size, M_BUSDMA, ((flags & BUS_DMA_NOWAIT) ?
	    M_NOWAIT : M_WAITOK) | M_ZERO, 0, dmat->lowaddr,
	    MAX(dmat->alignment, PAGE_SIZE), dmat->boundary);
	if (p == NULL) {
		bus_dmamap_destroy(dmat, map);
		*vaddr = NULL;
		return (ENOMEM);
	}
	map->mem = p;
	map->mem_pa = vtophys(p);
	/* out of the cache, then an uncached mapping for the CPU */
	dcache(p, size, 1);
	map->mem_nc = pmap_mapdev_attr(map->mem_pa, size, PAT_UNCACHED);
	map->coherent = 1;
	*vaddr = map->mem_nc;
	*mapp = map;
	return (0);
}

void
bus_dmamem_free(bus_dma_tag_t dmat, void *vaddr __unused, bus_dmamap_t map)
{
	bus_size_t size = round_page(dmat->maxsize);

	if (map == NULL)
		return;
	if (map->mem_nc != NULL)
		pmap_unmapdev((vm_offset_t)map->mem_nc, size);
	if (map->mem != NULL)
		contigfree(map->mem, size, M_BUSDMA);
	map->mem = map->mem_nc = NULL;
	bus_dmamap_destroy(dmat, map);
}

/* add [pa, pa + len) to the map's segments, merging when contiguous */
static int
add_seg(bus_dmamap_t map, vm_paddr_t pa, bus_size_t len)
{
	bus_dma_tag_t t = map->tag;

	while (len > 0) {
		bus_size_t chunk = len;
		bus_dma_segment_t *s;

		if (t->boundary) {
			bus_addr_t bmask = ~(t->boundary - 1);
			bus_addr_t next = (pa + t->boundary) & bmask;

			chunk = MIN(chunk, next - pa);
		}
		s = map->nsegs ? &map->segs[map->nsegs - 1] : NULL;
		if (s != NULL && s->ds_addr + s->ds_len == pa &&
		    s->ds_len + chunk <= t->maxsegsz &&
		    (t->boundary == 0 || (s->ds_addr & ~(t->boundary - 1)) ==
		    (pa & ~(t->boundary - 1)))) {
			s->ds_len += chunk;
		} else {
			if (map->nsegs >= t->nsegments)
				return (EFBIG);
			s = &map->segs[map->nsegs++];
			s->ds_addr = pa;
			s->ds_len = MIN(chunk, t->maxsegsz);
			chunk = s->ds_len;
		}
		pa += chunk;
		len -= chunk;
	}
	return (0);
}

/* does [va, va + len) lie wholly below the tag's lowaddr? */
static int
reachable(bus_dma_tag_t t, vm_offset_t va, bus_size_t len)
{
	vm_offset_t end = va + len;

	if (t->lowaddr == BUS_SPACE_MAXADDR)
		return (1);
	for (; va < end; va = trunc_page(va) + PAGE_SIZE)
		if (pmap_kextract(va) + MIN(end - va, PAGE_SIZE) - 1 >
		    t->lowaddr)
			return (0);
	return (1);
}

static int
load_piece(bus_dmamap_t map, void *buf, bus_size_t len)
{
	bus_dma_tag_t t = map->tag;
	vm_offset_t va = (vm_offset_t)buf;
	struct dma_piece *pc;
	int error;

	if (map->npieces >= MAX_PIECES)
		return (EFBIG);
	pc = &map->pieces[map->npieces++];
	pc->len = len;
	pc->orig = NULL;
	pc->kva = buf;
	if (map->coherent && map->mem != NULL) {
		/* the uncached mapping: segments from the physical pages */
		vm_offset_t off = va - (vm_offset_t)map->mem_nc;

		pc->kva = NULL;		/* no cache maintenance */
		return (add_seg(map, map->mem_pa + off, len));
	}
	if (!reachable(t, va, len)) {
		/* bounce: a buffer below lowaddr, as large as the tag allows */
		if (map->bounce == NULL) {
			map->bounce_size = round_page(t->maxsize);
			map->bounce = contigmalloc(map->bounce_size, M_BUSDMA,
			    M_NOWAIT, 0, t->lowaddr, PAGE_SIZE, t->boundary);
			if (map->bounce == NULL)
				return (ENOMEM);
			map->bounce_pa = vtophys(map->bounce);
		}
		if (map->bounce_used + len > map->bounce_size)
			return (EFBIG);
		pc->orig = buf;
		pc->kva = (char *)map->bounce + map->bounce_used;
		error = add_seg(map, map->bounce_pa + map->bounce_used, len);
		map->bounce_used += roundup2(len, 64);
		return (error);
	}
	while (len > 0) {
		bus_size_t chunk = MIN(len, PAGE_SIZE - (va & PAGE_MASK));

		error = add_seg(map, pmap_kextract(va), chunk);
		if (error)
			return (error);
		va += chunk;
		len -= chunk;
	}
	return (0);
}

static void
map_reset(bus_dmamap_t map)
{
	map->npieces = 0;
	map->nsegs = 0;
	map->bounce_used = 0;
}

int
bus_dmamap_load(bus_dma_tag_t dmat __unused, bus_dmamap_t map, void *buf,
    bus_size_t buflen, bus_dmamap_callback_t *callback, void *callback_arg,
    int flags __unused)
{
	int error;

	map_reset(map);
	error = load_piece(map, buf, buflen);
	if (callback != NULL)
		callback(callback_arg, map->segs, error ? 0 : map->nsegs, error);
	return (error);
}

static int
load_mbuf(bus_dmamap_t map, struct mbuf *m0)
{
	struct mbuf *m;
	int error = 0;

	map_reset(map);
	for (m = m0; m != NULL && error == 0; m = m->m_next)
		if (m->m_len > 0)
			error = load_piece(map, mtod(m, void *), m->m_len);
	return (error);
}

int
bus_dmamap_load_mbuf(bus_dma_tag_t dmat __unused, bus_dmamap_t map,
    struct mbuf *m0, bus_dmamap_callback2_t *callback, void *callback_arg,
    int flags __unused)
{
	int error = load_mbuf(map, m0);

	if (callback != NULL)
		callback(callback_arg, map->segs, error ? 0 : map->nsegs,
		    m0->m_pkthdr.len, error);
	return (error);
}

int
bus_dmamap_load_mbuf_segment(bus_dma_tag_t dmat __unused, bus_dmamap_t map,
    struct mbuf *m0, bus_dma_segment_t *segs, int maxsegs, int *nsegs,
    int flags __unused)
{
	int error = load_mbuf(map, m0);

	if (error == 0 && map->nsegs > maxsegs)
		error = EFBIG;
	if (error == 0) {
		memcpy(segs, map->segs, sizeof(*segs) * map->nsegs);
		*nsegs = map->nsegs;
	}
	return (error);
}

int
bus_dmamap_load_uio(bus_dma_tag_t dmat __unused, bus_dmamap_t map,
    struct uio *uio, bus_dmamap_callback2_t *callback, void *callback_arg,
    int flags __unused)
{
	bus_size_t total = 0;
	int error = 0, i;

	map_reset(map);
	if (uio->uio_segflg != UIO_SYSSPACE)
		error = EINVAL;
	for (i = 0; error == 0 && i < uio->uio_iovcnt; ++i) {
		error = load_piece(map, uio->uio_iov[i].iov_base,
		    uio->uio_iov[i].iov_len);
		total += uio->uio_iov[i].iov_len;
	}
	if (callback != NULL)
		callback(callback_arg, map->segs, error ? 0 : map->nsegs, total,
		    error);
	return (error);
}

int
bus_dmamap_load_ccb(bus_dma_tag_t dmat __unused, bus_dmamap_t map __unused,
    union ccb *ccb __unused, bus_dmamap_callback_t *callback __unused,
    void *callback_arg __unused, int flags __unused)
{
	return (ENOSYS);
}

void
_bus_dmamap_unload(bus_dma_tag_t dmat __unused, bus_dmamap_t map)
{
	map_reset(map);
}

void
_bus_dmamap_sync(bus_dma_tag_t dmat __unused, bus_dmamap_t map,
    bus_dmasync_op_t op)
{
	int i;

	__asm __volatile("dsb sy" : : : "memory");
	for (i = 0; i < map->npieces; ++i) {
		struct dma_piece *pc = &map->pieces[i];

		if (pc->kva == NULL)
			continue;
		if ((op & BUS_DMASYNC_PREWRITE) && pc->orig != NULL)
			memcpy(pc->kva, pc->orig, pc->len);
		if (op & (BUS_DMASYNC_PREWRITE | BUS_DMASYNC_PREREAD))
			dcache(pc->kva, pc->len, (op & BUS_DMASYNC_PREREAD) ? 1 : 0);
		if (op & BUS_DMASYNC_POSTREAD) {
			dcache(pc->kva, pc->len, 1);
			if (pc->orig != NULL)
				memcpy(pc->orig, pc->kva, pc->len);
		}
	}
}

void
busdma_swi(void)
{
}
