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
 * The first user program for DragonFly/arm64.
 *
 * There is no libc, runtime linker or cross toolchain for this port
 * yet, so this is a freestanding static executable that makes system
 * calls directly.  It runs as /sbin/init, exercises the parts of the
 * kernel a user process touches first (exec, system calls, demand
 * paging, fork, exit status, signals as a cause of death) and then
 * offers a tiny command prompt on the console.
 *
 * System call convention (see sys/platform/arm64/aarch64/trap.c):
 * number in x8, arguments in x0-x7, result in x0; on failure the carry
 * flag is set and x0 holds errno.  The wrapper below turns a failure
 * into a negative return value.
 */

#define	SYS_exit	1
#define	SYS_fork	2
#define	SYS_read	3
#define	SYS_write	4
#define	SYS_open	5
#define	SYS_wait4	7
#define	SYS_getpid	20
#define	SYS_reboot	55
#define	SYS_mmap	197
#define	SYS___sysctl	202
#define	SYS_clock_gettime 232

#define	O_RDWR		2
#define	PROT_RW		3
#define	MAP_PRIVATE	0x0002
#define	MAP_ANON	0x1000
#define	RB_AUTOBOOT	0
#define	RB_POWEROFF	0x4000
#define	CLOCK_MONOTONIC	4

typedef unsigned long size_t;

/*
 * Mark the executable as a native DragonFly binary; the kernel's ELF
 * loader looks for this note.
 */
__attribute__((section(".note.tag"), used, aligned(4)))
static const struct {
	int	namesz, descsz, type;
	char	name[12];
	int	desc;
} dragonfly_note = { 10, 4, 1, "DragonFly", 600522 };

static long
syscall(long num, long a0, long a1, long a2, long a3, long a4, long a5)
{
	register long x6 __asm__("x6") = 0;	/* seventh argument, if any */
	register long x8 __asm__("x8") = num;
	register long x0 __asm__("x0") = a0;
	register long x1 __asm__("x1") = a1;
	register long x2 __asm__("x2") = a2;
	register long x3 __asm__("x3") = a3;
	register long x4 __asm__("x4") = a4;
	register long x5 __asm__("x5") = a5;

	__asm__ volatile(
	    "svc	#0\n"
	    "csneg	x0, x0, x0, cc"
	    : "+r" (x0), "+r" (x1)
	    : "r" (x8), "r" (x2), "r" (x3), "r" (x4), "r" (x5), "r" (x6)
	    : "memory", "cc");
	return (x0);
}

#define	sys1(n, a)		syscall(n, (long)(a), 0, 0, 0, 0, 0)
#define	sys2(n, a, b)		syscall(n, (long)(a), (long)(b), 0, 0, 0, 0)
#define	sys3(n, a, b, c)	syscall(n, (long)(a), (long)(b), (long)(c), 0, 0, 0)

static size_t
strlen(const char *s)
{
	size_t n = 0;

	while (s[n] != '\0')
		++n;
	return (n);
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

static void
puts(const char *s)
{
	sys3(SYS_write, 1, s, strlen(s));
}

static void
putnum(long v)
{
	char buf[24];
	int i = sizeof(buf);
	unsigned long u = v < 0 ? -(unsigned long)v : (unsigned long)v;

	buf[--i] = '\0';
	do {
		buf[--i] = '0' + u % 10;
		u /= 10;
	} while (u != 0);
	if (v < 0)
		buf[--i] = '-';
	puts(&buf[i]);
}

static void
check(const char *what, int ok)
{
	puts(ok ? "  ok      " : "  FAILED  ");
	puts(what);
	puts("\n");
}

/* Fetch a string sysctl by its two-level numeric name. */
static void
print_sysctl(int top, int second)
{
	int mib[2] = { top, second };
	char buf[64];
	size_t len = sizeof(buf) - 1;

	if (syscall(SYS___sysctl, (long)mib, 2, (long)buf, (long)&len, 0, 0) < 0) {
		puts("?");
		return;
	}
	buf[len] = '\0';
	puts(buf);
}

static long
uptime_seconds(void)
{
	long ts[2] = { 0, 0 };

	sys2(SYS_clock_gettime, CLOCK_MONOTONIC, ts);
	return (ts[0]);
}

static int counter = 41;	/* lives in .data: tests copy-on-write */

/*
 * Fork a child that runs fn(), and return its wait status.
 */
static int
run_child(void (*fn)(void))
{
	int status = -1;
	long pid = sys1(SYS_fork, 0);

	if (pid == 0) {
		fn();
		sys1(SYS_exit, 0);
	}
	if (pid < 0)
		return (-1);
	if (syscall(SYS_wait4, pid, (long)&status, 0, 0, 0, 0) != pid)
		return (-1);
	return (status);
}

static void
child_exit7(void)
{
	counter = 1000;			/* must not be seen by the parent */
	sys1(SYS_exit, 7);
}

static void
child_null_deref(void)
{
	*(volatile int *)8 = 1;		/* dies of SIGSEGV */
}

static void
selftest(void)
{
	volatile char *p;
	int expect = counter + 1;
	int status;

	check("getpid() == 1", sys1(SYS_getpid, 0) == 1);

	++counter;
	check("write to .data", counter == expect);

	p = (volatile char *)syscall(SYS_mmap, 0, 3 * 4096, PROT_RW,
	    MAP_PRIVATE | MAP_ANON, -1, 0);
	check("mmap anonymous memory", (long)p > 0);
	if ((long)p > 0) {
		check("demand-zero page reads 0", p[4096] == 0);
		p[2 * 4096 + 5] = 'x';
		check("page fault on write", p[2 * 4096 + 5] == 'x');
	}

	status = run_child(child_exit7);
	check("fork, child exit status 7", status == (7 << 8));
	check("copy-on-write after fork", counter == expect);

	status = run_child(child_null_deref);
	check("child killed by SIGSEGV", (status & 0x7f) == 11);
}

static void
readline(char *buf, size_t size)
{
	size_t n = 0;
	char c;

	while (n < size - 1) {
		if (sys3(SYS_read, 0, &c, 1) != 1)
			continue;
		if (c == '\n')
			break;
		if (c == 0x7f || c == '\b') {
			if (n > 0) {
				--n;
				puts("\b \b");
			}
			continue;
		}
		buf[n++] = c;
		sys3(SYS_write, 1, &c, 1);	/* echo */
	}
	buf[n] = '\0';
	puts("\n");
}

static void
main(void)
{
	char line[64];

	/* init starts with no open files: console as stdin/out/err. */
	sys2(SYS_open, "/dev/console", O_RDWR);
	sys2(SYS_open, "/dev/console", O_RDWR);
	sys2(SYS_open, "/dev/console", O_RDWR);

	puts("\nHello from user mode on ");
	print_sysctl(1, 1);		/* kern.ostype */
	puts("/");
	print_sysctl(6, 1);		/* hw.machine */
	puts(" (pid ");
	putnum(sys1(SYS_getpid, 0));
	puts(")\n\n");

	selftest();

	puts("\nCommands: test, uptime, reboot, poweroff\n");
	for (;;) {
		puts("init> ");
		readline(line, sizeof(line));
		if (streq(line, "test")) {
			selftest();
		} else if (streq(line, "uptime")) {
			putnum(uptime_seconds());
			puts(" seconds\n");
		} else if (streq(line, "reboot")) {
			sys1(SYS_reboot, RB_AUTOBOOT);
		} else if (streq(line, "poweroff")) {
			sys1(SYS_reboot, RB_POWEROFF);
		} else if (line[0] != '\0') {
			puts("unknown command\n");
		}
	}
}

void
_start(void)
{
	main();
	sys1(SYS_exit, 0);
	for (;;)
		;
}
