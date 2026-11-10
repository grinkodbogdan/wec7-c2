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
 * Early polled console on an ARM PL011 UART, plus a small kprintf().
 *
 * This exists only so the platform code can report progress before the
 * machine-independent kernel (kern/subr_prf.c, kern/tty_cons.c) is
 * linked in.  Once it is, this file shrinks to a cn_tab entry providing
 * cnputc/cngetc and kprintf() comes from subr_prf.c.
 */

#include <sys/types.h>
#include <sys/_null.h>
#include <machine/stdarg.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	UART_DR		0x00
#define	UART_FR		0x18
#define	UART_FR_TXFF	0x20		/* transmit FIFO full */

int	kprintf(const char *fmt, ...) __printflike(1, 2);

static volatile uint32_t *uart;

void
cninit_early(vm_paddr_t uart_base)
{
	uart = (volatile uint32_t *)PHYS_TO_DMAP(uart_base);
}

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

static void
kputnum(unsigned long val, int base, int width, int zero, int neg)
{
	static const char digits[] = "0123456789abcdef";
	char buf[24];
	int i = 0;

	do {
		buf[i++] = digits[val % base];
		val /= base;
	} while (val != 0);
	if (neg)
		buf[i++] = '-';
	while (i < width--)
		cnputc_early(zero ? '0' : ' ');
	while (i > 0)
		cnputc_early(buf[--i]);
}

/*
 * Supports %c %s %d %u %x %p with the l/ll/j/z modifiers, field width
 * and zero padding.  Good enough for boot messages.
 */
int
kprintf(const char *fmt, ...)
{
	__va_list ap;
	const char *s;
	long sval;
	int c;

	__va_start(ap, fmt);
	while ((c = *fmt++) != '\0') {
		int lflag = 0;
		int width = 0;
		int zero = 0;

		if (c != '%') {
			cnputc_early(c);
			continue;
		}
		c = *fmt++;
		if (c == '0') {
			zero = 1;
			c = *fmt++;
		}
		while (c >= '0' && c <= '9') {
			width = width * 10 + (c - '0');
			c = *fmt++;
		}
		while (c == 'l' || c == 'j' || c == 'z') {
			lflag = 1;
			c = *fmt++;
		}
		switch (c) {
		case 'c':
			cnputc_early(__va_arg(ap, int));
			break;
		case 's':
			s = __va_arg(ap, const char *);
			if (s == NULL)
				s = "(null)";
			while (*s != '\0') {
				cnputc_early(*s++);
				--width;
			}
			while (width-- > 0)
				cnputc_early(' ');
			break;
		case 'd':
			sval = lflag ? __va_arg(ap, long) : __va_arg(ap, int);
			if (sval < 0)
				kputnum(-(unsigned long)sval, 10, width, zero, 1);
			else
				kputnum(sval, 10, width, zero, 0);
			break;
		case 'u':
			kputnum(lflag ? __va_arg(ap, unsigned long) :
			    __va_arg(ap, unsigned int), 10, width, zero, 0);
			break;
		case 'x':
			kputnum(lflag ? __va_arg(ap, unsigned long) :
			    __va_arg(ap, unsigned int), 16, width, zero, 0);
			break;
		case 'p':
			cnputc_early('0');
			cnputc_early('x');
			kputnum((unsigned long)__va_arg(ap, void *), 16,
			    0, 0, 0);
			break;
		case '%':
			cnputc_early('%');
			break;
		case '\0':
			--fmt;
			break;
		default:
			cnputc_early('%');
			cnputc_early(c);
			break;
		}
	}
	__va_end(ap);
	return (0);
}
