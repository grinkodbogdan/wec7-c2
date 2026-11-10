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

#ifndef _CPU_BUS_DMA_H_
#define _CPU_BUS_DMA_H_

#include <machine/cpufunc.h>

/*
 * Bus address and size types
 */
typedef uint64_t bus_addr_t;
typedef uint64_t bus_size_t;

typedef uint64_t bus_space_tag_t;
typedef uint64_t bus_space_handle_t;

#define BUS_SPACE_MAXSIZE_24BIT 0xFFFFFFUL
#define BUS_SPACE_MAXSIZE_32BIT	0xFFFFFFFFUL
#define BUS_SPACE_MAXSIZE	(64 * 1024) /* Maximum supported size */
#define BUS_SPACE_MAXADDR_24BIT	0xFFFFFFUL
#define BUS_SPACE_MAXADDR_32BIT	0xFFFFFFFFUL
#define BUS_SPACE_MAXADDR	0xFFFFFFFFFFFFFFFFUL

#define BUS_SPACE_UNRESTRICTED	(~0)	/* nsegments */

/*
 * There is no I/O port space on aarch64.  Every bus space is memory
 * mapped, a handle is simply the kernel virtual address of the mapping
 * and the tag is unused.  PCI I/O port windows, where a host bridge
 * provides one, are reached through a memory mapped window as well.
 */
#define	ARM64_BUS_SPACE_MEM	1

/*
 * Map a region of device bus space into CPU virtual address space.
 */
#define	BUS_SPACE_MAP_CACHEABLE		0x01
#define	BUS_SPACE_MAP_LINEAR		0x02

int bus_space_map(bus_space_tag_t, bus_addr_t, bus_size_t, int,
		  bus_space_handle_t *);

/*
 * Unmap a region of device bus space.
 */
void bus_space_unmap(bus_space_tag_t, bus_space_handle_t, bus_size_t);

/*
 * Get a new handle for a subregion of an already-mapped area of bus space.
 */
static __inline int
bus_space_subregion(bus_space_tag_t t __unused, bus_space_handle_t bsh,
		    bus_size_t offset, bus_size_t size __unused,
		    bus_space_handle_t *nbshp)
{
	*nbshp = bsh + offset;
	return (0);
}

static __inline void *
bus_space_kva(bus_space_tag_t tag __unused, bus_space_handle_t handle,
	      bus_size_t offset)
{
	return ((void *)(handle + offset));
}

/*
 * Single register access, repeated access to one register ("multi") and
 * access to consecutive registers ("region").  Device registers are
 * little-endian on every platform this port targets, as is the cpu, so
 * the "stream" (no byte swapping) variants are identical.
 *
 * Accesses go through volatile pointers; the compiler emits one load or
 * store of exactly the requested width for each.
 */
#define	__BUS_SPACE_ACCESSORS(N, TYPE)					\
static __inline TYPE							\
bus_space_read_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t handle, bus_size_t offset)			\
{									\
	return (*(volatile TYPE *)(handle + offset));			\
}									\
static __inline void							\
bus_space_write_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t handle, bus_size_t offset, TYPE value)		\
{									\
	*(volatile TYPE *)(handle + offset) = value;			\
}									\
static __inline void							\
bus_space_read_multi_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t bsh, bus_size_t offset, TYPE *addr, size_t count)\
{									\
	while (count-- > 0)						\
		*addr++ = *(volatile TYPE *)(bsh + offset);		\
}									\
static __inline void							\
bus_space_write_multi_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t bsh, bus_size_t offset, const TYPE *addr,	\
    size_t count)							\
{									\
	while (count-- > 0)						\
		*(volatile TYPE *)(bsh + offset) = *addr++;		\
}									\
static __inline void							\
bus_space_read_region_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t bsh, bus_size_t offset, TYPE *addr, size_t count)\
{									\
	volatile TYPE *p = (volatile TYPE *)(bsh + offset);		\
									\
	while (count-- > 0)						\
		*addr++ = *p++;						\
}									\
static __inline void							\
bus_space_write_region_##N(bus_space_tag_t tag __unused,		\
    bus_space_handle_t bsh, bus_size_t offset, const TYPE *addr,	\
    size_t count)							\
{									\
	volatile TYPE *p = (volatile TYPE *)(bsh + offset);		\
									\
	while (count-- > 0)						\
		*p++ = *addr++;						\
}									\
static __inline void							\
bus_space_set_multi_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t bsh, bus_size_t offset, TYPE value, size_t count)\
{									\
	while (count-- > 0)						\
		*(volatile TYPE *)(bsh + offset) = value;		\
}									\
static __inline void							\
bus_space_set_region_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t bsh, bus_size_t offset, TYPE value, size_t count)\
{									\
	volatile TYPE *p = (volatile TYPE *)(bsh + offset);		\
									\
	while (count-- > 0)						\
		*p++ = value;						\
}									\
/* Overlapping regions are handled like memmove(). */			\
static __inline void							\
bus_space_copy_region_##N(bus_space_tag_t tag __unused,			\
    bus_space_handle_t bsh1, bus_size_t off1, bus_space_handle_t bsh2,	\
    bus_size_t off2, size_t count)					\
{									\
	volatile TYPE *src = (volatile TYPE *)(bsh1 + off1);		\
	volatile TYPE *dst = (volatile TYPE *)(bsh2 + off2);		\
									\
	if ((bus_addr_t)src >= (bus_addr_t)dst) {			\
		while (count-- > 0)					\
			*dst++ = *src++;				\
	} else {							\
		src += count;						\
		dst += count;						\
		while (count-- > 0)					\
			*--dst = *--src;				\
	}								\
}

__BUS_SPACE_ACCESSORS(1, u_int8_t)
__BUS_SPACE_ACCESSORS(2, u_int16_t)
__BUS_SPACE_ACCESSORS(4, u_int32_t)
__BUS_SPACE_ACCESSORS(8, u_int64_t)

#undef __BUS_SPACE_ACCESSORS

#define	bus_space_read_stream_1		bus_space_read_1
#define	bus_space_read_stream_2		bus_space_read_2
#define	bus_space_read_stream_4		bus_space_read_4
#define	bus_space_read_stream_8		bus_space_read_8
#define	bus_space_write_stream_1	bus_space_write_1
#define	bus_space_write_stream_2	bus_space_write_2
#define	bus_space_write_stream_4	bus_space_write_4
#define	bus_space_write_stream_8	bus_space_write_8
#define	bus_space_read_multi_stream_1	bus_space_read_multi_1
#define	bus_space_read_multi_stream_2	bus_space_read_multi_2
#define	bus_space_read_multi_stream_4	bus_space_read_multi_4
#define	bus_space_write_multi_stream_1	bus_space_write_multi_1
#define	bus_space_write_multi_stream_2	bus_space_write_multi_2
#define	bus_space_write_multi_stream_4	bus_space_write_multi_4
#define	bus_space_read_region_stream_1	bus_space_read_region_1
#define	bus_space_read_region_stream_2	bus_space_read_region_2
#define	bus_space_read_region_stream_4	bus_space_read_region_4
#define	bus_space_write_region_stream_1	bus_space_write_region_1
#define	bus_space_write_region_stream_2	bus_space_write_region_2
#define	bus_space_write_region_stream_4	bus_space_write_region_4
#define	bus_space_set_multi_stream_1	bus_space_set_multi_1
#define	bus_space_set_multi_stream_2	bus_space_set_multi_2
#define	bus_space_set_multi_stream_4	bus_space_set_multi_4
#define	bus_space_set_region_stream_1	bus_space_set_region_1
#define	bus_space_set_region_stream_2	bus_space_set_region_2
#define	bus_space_set_region_stream_4	bus_space_set_region_4
#define	bus_space_copy_region_stream_1	bus_space_copy_region_1
#define	bus_space_copy_region_stream_2	bus_space_copy_region_2
#define	bus_space_copy_region_stream_4	bus_space_copy_region_4

/*
 * Bus read/write barrier methods.
 *
 * Device memory is mapped with a strongly ordered memory type, but
 * accesses to it are not ordered against accesses to normal memory
 * (DMA buffers, for example), so unlike x86 a barrier must emit a real
 * instruction.
 */
#define	BUS_SPACE_BARRIER_READ	0x01		/* force read barrier */
#define	BUS_SPACE_BARRIER_WRITE	0x02		/* force write barrier */

static __inline void
bus_space_barrier(bus_space_tag_t tag __unused,
    bus_space_handle_t bsh __unused, bus_size_t offset __unused,
    bus_size_t len __unused, int flags __unused)
{
	cpu_mfence();
}

#endif /* _CPU_BUS_DMA_H_ */
