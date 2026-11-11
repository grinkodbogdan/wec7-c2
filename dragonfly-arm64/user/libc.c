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
 * A minimal C library for DragonFly/arm64 test programs; see libc.h.
 *
 * System call convention (sys/platform/arm64/aarch64/trap.c): number
 * in x8, arguments in x0-x7, result in x0.  On failure the carry flag
 * is set and x0 holds the errno value.
 */

#include "libc.h"

#define	SYS_exit	1
#define	SYS_fork	2
#define	SYS_read	3
#define	SYS_write	4
#define	SYS_open	5
#define	SYS_close	6
#define	SYS_wait4	7
#define	SYS_unlink	10
#define	SYS_chdir	12
#define	SYS_getpid	20
#define	SYS_mount	21
#define	SYS_kill	37
#define	SYS_getppid	39
#define	SYS_ioctl	54
#define	SYS_reboot	55
#define	SYS_execve	59
#define	SYS_dup2	90
#define	SYS_mkdir	136
#define	SYS_rmdir	137
#define	SYS_setsid	147
#define	SYS_mmap	197
#define	SYS___sysctl	202
#define	SYS_clock_gettime 232
#define	SYS_nanosleep	240
#define	SYS___getcwd	326
#define	SYS_sigaction	342
#define	SYS_getdents	480

int	errno;
char	**environ;

int	main(int argc, char **argv, char **envp);

/* Marks the executable as a native DragonFly binary for the ELF loader. */
__attribute__((section(".note.tag"), used, aligned(4)))
static const struct {
	int	namesz, descsz, type;
	char	name[12];
	int	desc;
} dragonfly_note = { 10, 4, 1, "DragonFly", 600522 };

/*
 * long syscall(long num, a0 .. a6): shuffle the C arguments into the
 * kernel's convention.  fork() needs no special handling: the child
 * simply sees a return value of 0.
 */
__asm__(
"	.text\n"
"	.globl	syscall\n"
"	.type	syscall, %function\n"
"syscall:\n"
"	mov	x8, x0\n"
"	mov	x0, x1\n"
"	mov	x1, x2\n"
"	mov	x2, x3\n"
"	mov	x3, x4\n"
"	mov	x4, x5\n"
"	mov	x5, x6\n"
"	mov	x6, x7\n"
"	svc	#0\n"
"	b.cs	1f\n"
"	ret\n"
"1:	adrp	x1, errno\n"
"	str	w0, [x1, :lo12:errno]\n"
"	mov	x0, #-1\n"
"	ret\n"
/*
 * Process entry.  The kernel passes a pointer to the argument block
 * (argc, argv[], NULL, envp[], NULL) in x0.
 */
"	.globl	_start\n"
"	.type	_start, %function\n"
"_start:\n"
"	mov	x29, #0\n"
"	mov	x30, #0\n"
"	bl	__start_c\n"
);

void
__start_c(long *args)
{
	int argc = (int)args[0];
	char **argv = (char **)(args + 1);

	environ = argv + argc + 1;
	exit(main(argc, argv, environ));
}

void
_exit(int status)
{
	for (;;)
		syscall(SYS_exit, status);
}

void
exit(int status)
{
	_exit(status);
}

pid_t	fork(void) { return (syscall(SYS_fork)); }
pid_t	getpid(void) { return (syscall(SYS_getpid)); }
pid_t	getppid(void) { return (syscall(SYS_getppid)); }
int	close(int fd) { return (syscall(SYS_close, fd)); }
int	dup2(int from, int to) { return (syscall(SYS_dup2, from, to)); }
int	unlink(const char *path) { return (syscall(SYS_unlink, path)); }
int	mkdir(const char *path, int mode) { return (syscall(SYS_mkdir, path, mode)); }
int	rmdir(const char *path) { return (syscall(SYS_rmdir, path)); }
int	chdir(const char *path) { return (syscall(SYS_chdir, path)); }
int	kill(pid_t pid, int sig) { return (syscall(SYS_kill, pid, sig)); }
int	reboot(int howto) { return (syscall(SYS_reboot, howto)); }
int	setsid(void) { return (syscall(SYS_setsid)); }

pid_t
waitpid(pid_t pid, int *status, int options)
{
	return (syscall(SYS_wait4, pid, status, options, NULL));
}

int
execve(const char *path, char *const argv[], char *const envp[])
{
	return (syscall(SYS_execve, path, argv, envp));
}

int
open(const char *path, int flags, ...)
{
	va_list ap;
	int mode;

	va_start(ap, flags);
	mode = va_arg(ap, int);
	va_end(ap);
	return (syscall(SYS_open, path, flags, mode));
}

ssize_t
read(int fd, void *buf, size_t len)
{
	return (syscall(SYS_read, fd, buf, len));
}

ssize_t
write(int fd, const void *buf, size_t len)
{
	return (syscall(SYS_write, fd, buf, len));
}

int
ioctl(int fd, unsigned long cmd, void *arg)
{
	return (syscall(SYS_ioctl, fd, cmd, arg));
}

int
getdents(int fd, char *buf, size_t len)
{
	return (syscall(SYS_getdents, fd, buf, len));
}

int
mount(const char *type, const char *dir, int flags, void *data)
{
	return (syscall(SYS_mount, type, dir, flags, data));
}

char *
getcwd(char *buf, size_t size)
{
	return (syscall(SYS___getcwd, buf, size) < 0 ? NULL : buf);
}

/* struct sigaction from <sys/signal.h> */
struct sigaction {
	void		(*sa_handler)(int);
	int		sa_flags;
	unsigned int	sa_mask[4];
};

void (*
signal(int sig, void (*handler)(int)))(int)
{
	struct sigaction sa, osa;

	memset(&sa, 0, sizeof(sa));
	sa.sa_handler = handler;
	if (syscall(SYS_sigaction, sig, &sa, &osa) < 0)
		return ((void (*)(int))-1);
	return (osa.sa_handler);
}

int
sleep_ms(long ms)
{
	struct timespec ts = { ms / 1000, (ms % 1000) * 1000000 };

	return (syscall(SYS_nanosleep, &ts, NULL));
}

long
uptime_ms(void)
{
	struct timespec ts = { 0, 0 };

	syscall(SYS_clock_gettime, 4 /* CLOCK_MONOTONIC */, &ts);
	return (ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
}

int
sysctl_string(int top, int second, char *buf, size_t size)
{
	int mib[2] = { top, second };
	size_t len = size - 1;

	if (syscall(SYS___sysctl, mib, 2, buf, &len, NULL, 0) < 0) {
		buf[0] = '\0';
		return (-1);
	}
	buf[len] = '\0';
	return (0);
}

int
sysctl_int(int top, int second, int *val)
{
	int mib[2] = { top, second };
	size_t len = sizeof(*val);

	return (syscall(SYS___sysctl, mib, 2, val, &len, NULL, 0) < 0 ? -1 : 0);
}

/*
 * Read a sysctl by name: translate the name with the sysctl.name2oid
 * node (0.3), then read the oid.
 */
int
sysctlbyname(const char *name, void *oldp, size_t *oldlenp)
{
	int qoid[2] = { 0, 3 };
	int oid[24];
	size_t oidlen = sizeof(oid);

	if (syscall(SYS___sysctl, qoid, 2, oid, &oidlen, name,
	    strlen(name)) < 0)
		return (-1);
	return (syscall(SYS___sysctl, oid, oidlen / sizeof(int), oldp,
	    oldlenp, NULL, 0) < 0 ? -1 : 0);
}

void *
mmap_anon(size_t len)
{
	long p = syscall(SYS_mmap, NULL, len, 3 /* PROT_READ|PROT_WRITE */,
	    0x1002 /* MAP_ANON|MAP_PRIVATE */, -1, 0, 0L);

	return (p == -1 ? NULL : (void *)p);
}

size_t
strlen(const char *s)
{
	size_t n = 0;

	while (s[n] != '\0')
		++n;
	return (n);
}

int
strcmp(const char *a, const char *b)
{
	while (*a != '\0' && *a == *b) {
		++a;
		++b;
	}
	return ((unsigned char)*a - (unsigned char)*b);
}

int
strncmp(const char *a, const char *b, size_t n)
{
	for (; n > 0; --n, ++a, ++b) {
		if (*a != *b || *a == '\0')
			return ((unsigned char)*a - (unsigned char)*b);
	}
	return (0);
}

char *
strcpy(char *dst, const char *src)
{
	char *d = dst;

	while ((*d++ = *src++) != '\0')
		;
	return (dst);
}

char *
strchr(const char *s, int c)
{
	for (; *s != '\0'; ++s) {
		if (*s == (char)c)
			return ((char *)s);
	}
	return (NULL);
}

void *
memcpy(void *dst, const void *src, size_t n)
{
	char *d = dst;
	const char *s = src;

	while (n-- > 0)
		*d++ = *s++;
	return (dst);
}

void *
memset(void *dst, int c, size_t n)
{
	char *d = dst;

	while (n-- > 0)
		*d++ = (char)c;
	return (dst);
}

long
atol(const char *s)
{
	long v = 0;
	int neg = (*s == '-');

	if (neg)
		++s;
	while (*s >= '0' && *s <= '9')
		v = v * 10 + (*s++ - '0');
	return (neg ? -v : v);
}

/*
 * malloc: carve allocations off 64KB chunks of anonymous memory.
 * free() does nothing; these programs are short lived.
 */
void *
malloc(size_t size)
{
	static char *cur, *end;
	void *p;

	size = (size + 15) & ~15UL;
	if (cur == NULL || (size_t)(end - cur) < size) {
		size_t chunk = size > 65536 ? size : 65536;

		cur = mmap_anon(chunk);
		if (cur == NULL)
			return (NULL);
		end = cur + chunk;
	}
	p = cur;
	cur += size;
	return (p);
}

void
free(void *ptr)
{
	(void)ptr;
}

struct outbuf {
	int	fd;
	int	len;
	int	total;
	char	buf[256];
};

static void
out_flush(struct outbuf *o)
{
	if (o->len > 0)
		write(o->fd, o->buf, o->len);
	o->len = 0;
}

static void
out_char(struct outbuf *o, char c)
{
	if (o->len == (int)sizeof(o->buf))
		out_flush(o);
	o->buf[o->len++] = c;
	++o->total;
}

static void
out_num(struct outbuf *o, unsigned long v, int base, int width, int zero,
	int neg)
{
	char tmp[24];
	int i = 0;

	do {
		tmp[i++] = "0123456789abcdef"[v % base];
		v /= base;
	} while (v != 0);
	if (neg)
		tmp[i++] = '-';
	while (i < width--)
		out_char(o, zero ? '0' : ' ');
	while (i > 0)
		out_char(o, tmp[--i]);
}

/*
 * %c %s %d %u %x %p %f %%, with l/z modifiers, field width, zero
 * padding, left justification for %s and a precision for %f.
 */
static void
vformat(struct outbuf *o, const char *fmt, va_list ap)
{
	const char *s;
	double d;
	long v;
	int c;

	while ((c = *fmt++) != '\0') {
		int lflag = 0, width = 0, zero = 0, left = 0, prec = 6;

		if (c != '%') {
			out_char(o, c);
			continue;
		}
		c = *fmt++;
		if (c == '-') {
			left = 1;
			c = *fmt++;
		}
		if (c == '0') {
			zero = 1;
			c = *fmt++;
		}
		while (c >= '0' && c <= '9') {
			width = width * 10 + (c - '0');
			c = *fmt++;
		}
		if (c == '.') {
			prec = 0;
			c = *fmt++;
			while (c >= '0' && c <= '9') {
				prec = prec * 10 + (c - '0');
				c = *fmt++;
			}
		}
		while (c == 'l' || c == 'z') {
			lflag = 1;
			c = *fmt++;
		}
		switch (c) {
		case 'c':
			out_char(o, va_arg(ap, int));
			break;
		case 's':
			s = va_arg(ap, const char *);
			if (s == NULL)
				s = "(null)";
			width -= (int)strlen(s);
			while (!left && width-- > 0)
				out_char(o, ' ');
			while (*s != '\0')
				out_char(o, *s++);
			while (left && width-- > 0)
				out_char(o, ' ');
			break;
		case 'd':
			v = lflag ? va_arg(ap, long) : va_arg(ap, int);
			out_num(o, v < 0 ? -(unsigned long)v : (unsigned long)v,
			    10, width, zero, v < 0);
			break;
		case 'u':
			out_num(o, lflag ? va_arg(ap, unsigned long) :
			    va_arg(ap, unsigned int), 10, width, zero, 0);
			break;
		case 'x':
			out_num(o, lflag ? va_arg(ap, unsigned long) :
			    va_arg(ap, unsigned int), 16, width, zero, 0);
			break;
		case 'p':
			out_char(o, '0');
			out_char(o, 'x');
			out_num(o, (unsigned long)va_arg(ap, void *), 16, 0,
			    0, 0);
			break;
		case 'f':
			d = va_arg(ap, double);
			if (d < 0) {
				out_char(o, '-');
				d = -d;
			}
			out_num(o, (unsigned long)d, 10, 0, 0, 0);
			if (prec > 0)
				out_char(o, '.');
			d -= (double)(unsigned long)d;
			while (prec-- > 0) {
				d *= 10.0;
				out_char(o, '0' + (int)d);
				d -= (int)d;
			}
			break;
		case '%':
			out_char(o, '%');
			break;
		case '\0':
			--fmt;
			break;
		default:
			out_char(o, '%');
			out_char(o, c);
			break;
		}
	}
	out_flush(o);
}

int
printf(const char *fmt, ...)
{
	struct outbuf o = { .fd = 1 };
	va_list ap;

	va_start(ap, fmt);
	vformat(&o, fmt, ap);
	va_end(ap);
	return (o.total);
}

int
dprintf(int fd, const char *fmt, ...)
{
	struct outbuf o = { .fd = fd };
	va_list ap;

	va_start(ap, fmt);
	vformat(&o, fmt, ap);
	va_end(ap);
	return (o.total);
}

int
puts(const char *s)
{
	return (printf("%s\n", s));
}

/*
 * Read one line, without its newline.  Returns its length, or -1 at
 * end of file.  The terminal's line discipline does the editing.
 */
int
getline_fd(int fd, char *buf, size_t size)
{
	size_t n = 0;
	char c;
	ssize_t r;

	for (;;) {
		r = read(fd, &c, 1);
		if (r < 0 && errno == EINTR) {
			n = 0;		/* interrupted: start the line over */
			buf[0] = '\0';
			return (0);
		}
		if (r <= 0)
			return (n > 0 ? (int)n : -1);
		if (c == '\n')
			break;
		if (n < size - 1)
			buf[n++] = c;
	}
	buf[n] = '\0';
	return ((int)n);
}
