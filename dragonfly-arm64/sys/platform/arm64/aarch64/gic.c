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
 * ARM Generic Interrupt Controller v2 (GIC-400 and friends).
 *
 * Only the boot cpu's interface is initialized and every interrupt is
 * routed to it.  GICv3, which anything newer than roughly 2016-era
 * SoCs uses, is not supported yet.
 */

#include <sys/param.h>
#include <sys/systm.h>

#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	GICD_CTLR		0x000
#define	GICD_TYPER		0x004
#define	GICD_ISENABLER(n)	(0x100 + 4 * ((n) / 32))
#define	GICD_ICENABLER(n)	(0x180 + 4 * ((n) / 32))
#define	GICD_IPRIORITYR(n)	(0x400 + (n))
#define	GICD_ITARGETSR(n)	(0x800 + (n))

#define	GICC_CTLR		0x000
#define	GICC_PMR		0x004
#define	GICC_IAR		0x00c
#define	GICC_EOIR		0x010

#define	GIC_SPURIOUS		1023
#define	GIC_FIRST_SPI		32

static volatile uint8_t *gicd;
static volatile uint8_t *gicc;
static int gic_nirqs;

static __inline uint32_t
gic_read(volatile uint8_t *base, u_int off)
{
	return (*(volatile uint32_t *)(base + off));
}

static __inline void
gic_write(volatile uint8_t *base, u_int off, uint32_t val)
{
	*(volatile uint32_t *)(base + off) = val;
}

/*
 * Returns the number of interrupt lines the distributor implements.
 */
int
gic_init(vm_paddr_t gicd_pa, vm_paddr_t gicc_pa)
{
	int irq;

	gicd = (volatile uint8_t *)PHYS_TO_DMAP(gicd_pa);
	gicc = (volatile uint8_t *)PHYS_TO_DMAP(gicc_pa);
	gic_nirqs = 32 * ((gic_read(gicd, GICD_TYPER) & 0x1f) + 1);

	/* Everything off, default priority, shared interrupts to cpu 0. */
	for (irq = 0; irq < gic_nirqs; irq += 32)
		gic_write(gicd, GICD_ICENABLER(irq), 0xffffffff);
	for (irq = 0; irq < gic_nirqs; ++irq) {
		gicd[GICD_IPRIORITYR(irq)] = 0x80;
		if (irq >= GIC_FIRST_SPI)
			gicd[GICD_ITARGETSR(irq)] = 0x01;
	}

	gic_write(gicd, GICD_CTLR, 1);		/* enable distributor */
	gic_write(gicc, GICC_PMR, 0xff);	/* unmask all priorities */
	gic_write(gicc, GICC_CTLR, 1);		/* enable cpu interface */
	return (gic_nirqs);
}

void
gic_enable_irq(int irq)
{
	if (gicd != NULL && irq < gic_nirqs)
		gic_write(gicd, GICD_ISENABLER(irq), 1U << (irq % 32));
}

void
gic_disable_irq(int irq)
{
	if (gicd != NULL && irq < gic_nirqs)
		gic_write(gicd, GICD_ICENABLER(irq), 1U << (irq % 32));
}

/*
 * Acknowledge the highest priority pending interrupt.  Returns the raw
 * IAR value (to be handed back to gic_eoi()) or -1 if there is none.
 */
int
gic_ack(void)
{
	uint32_t iar;

	if (gicc == NULL)
		return (-1);
	iar = gic_read(gicc, GICC_IAR);
	if ((iar & 0x3ff) == GIC_SPURIOUS)
		return (-1);
	return ((int)iar);
}

void
gic_eoi(int iar)
{
	gic_write(gicc, GICC_EOIR, (uint32_t)iar);
}
