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
 * The system console: an ARM PL011 UART, a firmware framebuffer, or
 * both at once.
 *
 * The kernel console half (kprintf, the mountroot prompt) polls the
 * UART's data register directly and mirrors output to the framebuffer
 * (fbcons.c) when there is one.  The device node, /dev/ttyu0 and
 * through it /dev/console, is a real tty: line discipline, termios,
 * job control signals from ^C and ^Z, window size.  Transmit is
 * synchronous and receive is polled from a callout 50 times a second
 * rather than interrupt driven, which keeps the driver independent of
 * how a given board wires the UART's interrupt.
 *
 * A machine with only a framebuffer (a phone) gets output and no
 * input: there is no keyboard driver.
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
#include <sys/uio.h>
#include <sys/tty.h>
#include <sys/ttydefaults.h>
#include <sys/callout.h>
#include <sys/fcntl.h>
#include <sys/proc.h>
#include <sys/caps.h>

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

static d_open_t		pl011_open;
static d_close_t	pl011_close;
static d_ioctl_t	pl011_ioctl;

static struct dev_ops pl011_ops = {
	{ "pl011", 0, D_TTY },
	.d_open =	pl011_open,
	.d_close =	pl011_close,
	.d_read =	ttyread,
	.d_write =	ttywrite,
	.d_ioctl =	pl011_ioctl,
	.d_kqfilter =	ttykqfilter
};

static struct callout pl011_poll_callout;
static struct tty *pl011_tty;

/*
 * Called from initarm() before anything else can print.
 */
void
cninit_early(vm_paddr_t uart_base)
{
	if (uart_base != 0)
		uart = (volatile uint32_t *)PHYS_TO_DMAP(uart_base);
}

/*
 * Raw output for the paths that cannot use kprintf(): before cninit()
 * and while reporting a fault in the console code itself.
 */
void
cnputc_early(int c)
{
	if (uart != NULL) {
		if (c == '\n')
			cnputc_early('\r');
		while (uart[UART_FR / 4] & UART_FR_TXFF)
			;
		uart[UART_DR / 4] = (uint32_t)c;
	}
	if (c != '\r' || uart == NULL)
		fbcons_putc(c);
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
	if (uart == NULL && !fbcons_present()) {
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
	if (uart == NULL || (uart[UART_FR / 4] & UART_FR_RXFE))
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

/*
 * Start output: drain the tty's output queue into the transmitter.
 */
static void
pl011_tty_start(struct tty *tp)
{
	char buf[64];
	int n, i;

	lwkt_gettoken(&tp->t_token);
	if (tp->t_state & (TS_TIMEOUT | TS_TTSTOP)) {
		ttwwakeup(tp);
		lwkt_reltoken(&tp->t_token);
		return;
	}
	tp->t_state |= TS_BUSY;
	while ((n = clist_qtob(&tp->t_outq, buf, sizeof(buf))) > 0) {
		for (i = 0; i < n; ++i) {
			if (uart != NULL) {
				while (uart[UART_FR / 4] & UART_FR_TXFF)
					;
				uart[UART_DR / 4] = (uint32_t)(u_char)buf[i];
			}
			fbcons_putc((u_char)buf[i]);
		}
	}
	tp->t_state &= ~TS_BUSY;
	lwkt_reltoken(&tp->t_token);
	ttwwakeup(tp);
}

/* The firmware configured the line; accept whatever is asked for. */
static int
pl011_tty_param(struct tty *tp, struct termios *tio)
{
	lwkt_gettoken(&tp->t_token);
	tp->t_ispeed = tio->c_ispeed;
	tp->t_ospeed = tio->c_ospeed;
	tp->t_cflag = tio->c_cflag;
	lwkt_reltoken(&tp->t_token);
	return (0);
}

/*
 * Feed received characters to the line discipline.
 */
static void
pl011_poll(void *arg __unused)
{
	struct tty *tp = pl011_tty;
	int c;

	if (tp != NULL) {
		lwkt_gettoken(&tp->t_token);
		if (tp->t_state & TS_ISOPEN) {
			while ((c = pl011_cncheckc(NULL)) != -1)
				(*linesw[tp->t_line].l_rint)(c, tp);
		}
		lwkt_reltoken(&tp->t_token);
	}
	callout_reset(&pl011_poll_callout, hz / 50 + 1, pl011_poll, NULL);
}

static int
pl011_open(struct dev_open_args *ap)
{
	cdev_t dev = ap->a_head.a_dev;
	struct tty *tp;
	int error;

	tp = ttymalloc(&dev->si_tty);
	lwkt_gettoken(&tp->t_token);
	if ((tp->t_state & TS_ISOPEN) == 0) {
		tp->t_oproc = pl011_tty_start;
		tp->t_param = pl011_tty_param;
		tp->t_stop = nottystop;
		tp->t_dev = dev;

		tp->t_state |= TS_CARR_ON | TS_CONNECTED;
		ttychars(tp);
		tp->t_iflag = TTYDEF_IFLAG;
		tp->t_oflag = TTYDEF_OFLAG;
		tp->t_cflag = TTYDEF_CFLAG | CLOCAL;
		tp->t_lflag = TTYDEF_LFLAG;
		tp->t_ispeed = TTYDEF_SPEED;
		tp->t_ospeed = TTYDEF_SPEED;
		tp->t_winsize.ws_row = 24;
		tp->t_winsize.ws_col = 80;
		ttsetwater(tp);
	} else if ((tp->t_state & TS_XCLUDE) &&
		   caps_priv_check(ap->a_cred, SYSCAP_RESTRICTEDROOT)) {
		lwkt_reltoken(&tp->t_token);
		return (EBUSY);
	}
	error = (*linesw[tp->t_line].l_open)(dev, tp);
	if (error == 0 && pl011_tty == NULL) {
		pl011_tty = tp;
		callout_init_mp(&pl011_poll_callout);
		callout_reset(&pl011_poll_callout, hz / 50 + 1, pl011_poll,
		    NULL);
	}
	lwkt_reltoken(&tp->t_token);
	return (error);
}

static int
pl011_close(struct dev_close_args *ap)
{
	cdev_t dev = ap->a_head.a_dev;
	struct tty *tp = dev->si_tty;

	lwkt_gettoken(&tp->t_token);
	(*linesw[tp->t_line].l_close)(tp, ap->a_fflag);
	ttyclose(tp);
	lwkt_reltoken(&tp->t_token);
	return (0);
}

static int
pl011_ioctl(struct dev_ioctl_args *ap)
{
	cdev_t dev = ap->a_head.a_dev;
	struct tty *tp = dev->si_tty;
	int error;

	lwkt_gettoken(&tp->t_token);
	error = (*linesw[tp->t_line].l_ioctl)(tp, ap->a_cmd, ap->a_data,
					      ap->a_fflag, ap->a_cred);
	if (error == ENOIOCTL)
		error = ttioctl(tp, ap->a_cmd, ap->a_data, ap->a_fflag);
	lwkt_reltoken(&tp->t_token);
	return (error == ENOIOCTL ? ENOTTY : error);
}
