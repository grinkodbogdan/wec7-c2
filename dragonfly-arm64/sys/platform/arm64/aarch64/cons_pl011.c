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
 * Polled console on an ARM PL011 UART.
 *
 * This is a kernel console only: output and blocking input by polling
 * the data register.  There is no tty layer attachment or interrupt
 * driven input yet, so nothing in userland could log in over it; the
 * device node exists because the console framework requires one.
 *
 * The UART is assumed to have been configured (baud rate, enabled) by
 * the firmware, and to sit in the low 1GB that locore maps as device
 * memory through the direct map.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/conf.h>
#include <sys/cons.h>
#include <sys/device.h>

#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	UART_DR		0x00
#define	UART_FR		0x18
#define	UART_FR_RXFE	0x10		/* receive FIFO empty */
#define	UART_FR_TXFF	0x20		/* transmit FIFO full */

static volatile uint32_t *uart;

static cn_probe_t	pl011_cnprobe;
static cn_init_t	pl011_cninit;
static cn_init_fini_t	pl011_cninit_fini;
static cn_getc_t	pl011_cngetc;
static cn_checkc_t	pl011_cncheckc;
static cn_putc_t	pl011_cnputc;

CONS_DRIVER(pl011, pl011_cnprobe, pl011_cninit, pl011_cninit_fini, NULL,
	    pl011_cngetc, pl011_cncheckc, pl011_cnputc, NULL, NULL);

static struct dev_ops pl011_ops = {
	{ "pl011", 0, 0 }
};

/*
 * Called from initarm() before anything else can print.
 */
void
cninit_early(vm_paddr_t uart_base)
{
	uart = (volatile uint32_t *)PHYS_TO_DMAP(uart_base);
}

/*
 * Raw output for the paths that cannot use kprintf(): before cninit()
 * and while reporting a fault in the console code itself.
 */
void
cnputc_early(int c)
{
	if (uart == NULL)
		return;
	if (c == '\n')
		cnputc_early('\r');
	while (uart[UART_FR / 4] & UART_FR_TXFF)
		;
	uart[UART_DR / 4] = (uint32_t)c;
}

void
cnputs_early(const char *s)
{
	while (*s != '\0')
		cnputc_early(*s++);
}

static void
pl011_cnprobe(struct consdev *cp)
{
	if (uart == NULL) {
		cp->cn_pri = CN_DEAD;
		return;
	}
	cp->cn_pri = CN_REMOTE;
	cp->cn_probegood = 1;
}

static void
pl011_cninit(struct consdev *cp __unused)
{
}

static void
pl011_cninit_fini(struct consdev *cp)
{
	cp->cn_dev = make_dev(&pl011_ops, 0, UID_ROOT, GID_WHEEL, 0600,
	    "ttyu0");
}

static void
pl011_cnputc(void *private __unused, int c)
{
	cnputc_early(c);
}

static int
pl011_cncheckc(void *private __unused)
{
	if (uart[UART_FR / 4] & UART_FR_RXFE)
		return (-1);
	return (uart[UART_DR / 4] & 0xff);
}

static int
pl011_cngetc(void *private)
{
	int c;

	while ((c = pl011_cncheckc(private)) == -1)
		cpu_pause();
	return (c);
}
