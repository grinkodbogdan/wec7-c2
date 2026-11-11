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
 * /sbin/init for the DragonFly/arm64 test ramdisk.
 *
 * Sets up the console as the controlling terminal, mounts a tmpfs on
 * /tmp, runs a self-test of what a user process needs from the kernel,
 * then keeps a shell running on the console.  Built against the mini
 * C library in this directory (see libc.h for why).
 */

#include "libc.h"

#define	TIOCSCTTY	0x20007461UL	/* _IO('t', 97) */

static int failures;
static int counter = 41;		/* in .data: tests copy-on-write */
static volatile int got_signal;
static volatile double handler_fp;

static void
check(const char *what, int ok)
{
	printf("  %s  %s\n", ok ? "ok    " : "FAILED", what);
	if (!ok)
		++failures;
}

/* Fork a child that runs fn(), and return its wait status. */
static int
run_child(void (*fn)(void))
{
	int status = -1;
	pid_t pid = fork();

	if (pid == 0) {
		fn();
		_exit(0);
	}
	if (pid < 0 || waitpid(pid, &status, 0) != pid)
		return (-1);
	return (status);
}

static void
child_exit7(void)
{
	counter = 1000;			/* must not be seen by the parent */
	_exit(7);
}

static void
child_null_deref(void)
{
	*(volatile int *)8 = 1;		/* dies of SIGSEGV */
}

/* Keeps the FP registers busy with values the parent does not use. */
static void
child_fp_noise(void)
{
	volatile double x = 12345.678;
	long end = uptime_ms() + 150;

	while (uptime_ms() < end)
		x = x * 1.000001 + 0.5;
}

/*
 * A fixed amount of integer work, a few seconds on one emulated core.
 * Long enough that the scheduler's delay in spreading freshly forked
 * processes over the cpus does not dominate the measurement.
 */
static void
child_spin(void)
{
	volatile unsigned long x = 0;
	unsigned long i;

	for (i = 0; i < 450000000UL; ++i)
		x += i ^ (x >> 3);
	_exit(0);
}

/* Fork and reap many short-lived children. */
static void
child_forker(void)
{
	int i, status, bad = 0;
	pid_t pid;

	for (i = 0; i < 100; ++i) {
		pid = fork();
		if (pid == 0)
			_exit(i & 0x7f);
		if (pid < 0 || waitpid(pid, &status, 0) != pid ||
		    !WIFEXITED(status) || WEXITSTATUS(status) != (i & 0x7f))
			++bad;
	}
	_exit(bad != 0);
}

/* Start n children running fn at once; return how many exited 0. */
static int
run_parallel(int n, void (*fn)(void))
{
	pid_t pids[16];
	int i, status, good = 0;

	if (n > 16)
		n = 16;
	for (i = 0; i < n; ++i) {
		pids[i] = fork();
		if (pids[i] == 0)
			fn();
	}
	for (i = 0; i < n; ++i) {
		if (pids[i] > 0 && waitpid(pids[i], &status, 0) == pids[i] &&
		    WIFEXITED(status) && WEXITSTATUS(status) == 0)
			++good;
	}
	return (good);
}

/*
 * With more than one cpu: show that they share the work, and run the
 * fork path on all of them at once.
 */
/*
 * Print each cpu's clock (measured by the kernel as the cpu started)
 * and how many timer interrupts each takes per second (a few hundred
 * is normal; tens of thousands is a storm).  Returns nonzero if a cpu runs at under a quarter of cpu0's
 * speed, where the timing test would take minutes.
 */
static int
cpu_report(int ncpu)
{
	static int mhz[256];
	static unsigned long t0[256], t1[256];
	size_t len = sizeof(mhz), l0 = sizeof(t0), l1 = sizeof(t1);
	int i, slow = 0;

	if (ncpu > 64)
		ncpu = 64;
	if (sysctlbyname("machdep.cpu_mhz", mhz, &len) != 0)
		return (0);
	printf("  clock, MHz:      ");
	for (i = 0; i < ncpu; ++i) {
		printf(" cpu%d %d", i, mhz[i]);
		if (mhz[i] * 4 < mhz[0])
			slow = 1;
	}
	printf("\n");
	if (sysctlbyname("machdep.timer_intrs", t0, &l0) == 0) {
		sleep_ms(1000);
		sysctlbyname("machdep.timer_intrs", t1, &l1);
		printf("  timer intr/s:    ");
		for (i = 0; i < ncpu; ++i)
			printf(" cpu%d %lu", i, t1[i] - t0[i]);
		printf("\n");
	}
	return (slow);
}

static void
smptest(void)
{
	long t0, t1, tn;
	int ncpu = 1;

	sysctl_int(6, 3, &ncpu);		/* hw.ncpu */
	printf("  cpus online: %d\n", ncpu);
	if (ncpu < 2)
		return;
	if (cpu_report(ncpu) != 0) {
		printf("  some cpus are far slower than cpu0: skipping the "
		    "timing test\n");
		++failures;
		check("fork storm on all cpus (4 x 100 children)",
		    run_parallel(4, child_forker) == 4);
		return;
	}

	t0 = uptime_ms();
	run_parallel(1, child_spin);
	t1 = uptime_ms() - t0;
	t0 = uptime_ms();
	check("one busy process per cpu", run_parallel(ncpu, child_spin) == ncpu);
	tn = uptime_ms() - t0;
	printf("  1 busy process: %ld ms, %d at once: %ld ms (%ld.%ldx)\n",
	    t1, ncpu, tn, ncpu * t1 / (tn ? tn : 1),
	    (ncpu * t1 * 10 / (tn ? tn : 1)) % 10);
	/* No parallelism measures 1.0x or less; ask for clearly more. */
	check("cpus run in parallel", tn * 13 < ncpu * t1 * 10);

	check("fork storm on all cpus (4 x 100 children)",
	    run_parallel(4, child_forker) == 4);
}

static void
on_usr1(int sig)
{
	/* Use FP in the handler: the interrupted code's must survive. */
	handler_fp = (double)sig * 2.5;
	got_signal = sig;
}

static void
selftest(void)
{
	volatile char *p;
	volatile double sum = 0.0;
	char buf[32];
	int expect = counter + 1;
	int status, fd, i;
	pid_t pid;

	failures = 0;
	++counter;
	check("write to .data", counter == expect);

	p = mmap_anon(3 * 4096);
	check("mmap anonymous memory", p != NULL);
	if (p != NULL) {
		check("demand-zero page reads 0", p[4096] == 0);
		p[2 * 4096 + 5] = 'x';
		check("page fault on write", p[2 * 4096 + 5] == 'x');
	}

	status = run_child(child_exit7);
	check("fork, child exit status 7",
	    WIFEXITED(status) && WEXITSTATUS(status) == 7);
	check("copy-on-write after fork", counter == expect);

	status = run_child(child_null_deref);
	check("child killed by SIGSEGV",
	    WIFSIGNALED(status) && WTERMSIG(status) == SIGSEGV);

	/* A handler runs and the interrupted code carries on. */
	got_signal = 0;
	signal(SIGUSR1, on_usr1);
	sum = 1.25;
	kill(getpid(), SIGUSR1);
	check("signal handler ran", got_signal == SIGUSR1);
	check("handler could use floating point", handler_fp == 75.0);
	check("floating point intact after handler", sum == 1.25);
	signal(SIGUSR1, SIG_DFL);

	/*
	 * Floating point state is per process: run a long computation
	 * while a child scribbles on the same registers, with sleeps to
	 * force switches between the two.
	 */
	pid = fork();
	if (pid == 0) {
		child_fp_noise();
		_exit(0);
	}
	sum = 0.0;
	for (i = 0; i < 100; ++i) {
		int j;

		for (j = 0; j < 2000; ++j)
			sum += 0.5;
		sleep_ms(1);
	}
	waitpid(pid, &status, 0);
	check("floating point across context switches", sum == 100000.0);

	/* A writable filesystem. */
	fd = open("/tmp/selftest", O_RDWR | O_CREAT | O_TRUNC, 0644);
	check("create a file on tmpfs", fd >= 0);
	if (fd >= 0) {
		write(fd, "dragonfly", 9);
		close(fd);
		fd = open("/tmp/selftest", O_RDONLY);
		memset(buf, 0, sizeof(buf));
		check("read it back",
		    read(fd, buf, sizeof(buf)) == 9 &&
		    strcmp(buf, "dragonfly") == 0);
		close(fd);
		check("remove it", unlink("/tmp/selftest") == 0);
	}

	smptest();

	if (failures == 0)
		printf("All checks passed.\n");
	else
		printf("%d check(s) FAILED.\n", failures);
}

int
main(void)
{
	char ostype[32], machine[32];
	char *sh_argv[] = { "sh", "-i", NULL };
	char *sh_env[] = { "PATH=/bin:/sbin:/usr/bin:/usr/sbin", "HOME=/",
	    "TERM=vt100", "PS1=# ", NULL };
	int status;
	pid_t pid;

	if (getpid() != 1) {
		/* Run by hand from the shell: just the tests. */
		selftest();
		return (failures != 0);
	}

	/* init starts with no open files and no terminal. */
	setsid();
	open("/dev/console", O_RDWR);
	dup2(0, 1);
	dup2(0, 2);
	ioctl(0, TIOCSCTTY, NULL);

	sysctl_string(1, 1, ostype, sizeof(ostype));	/* kern.ostype */
	sysctl_string(6, 1, machine, sizeof(machine));	/* hw.machine */
	printf("\nHello from user mode on %s/%s\n\n", ostype, machine);

	if (mount("tmpfs", "/tmp", 0, NULL) != 0)
		printf("init: cannot mount tmpfs on /tmp (errno %d)\n", errno);
	/* /var is a link to /tmp/var on the read-only ramdisk. */
	mkdir("/tmp/var", 0755);
	mkdir("/tmp/var/run", 0755);
	mkdir("/tmp/var/run/sem", 01777);
	mkdir("/tmp/var/run/shm", 01777);
	mkdir("/tmp/var/tmp", 01777);

	selftest();

	/*
	 * A script of DragonFly's own programs, for a screen nobody can
	 * type on yet (the Mi 5 has no input driver).
	 */
	if (access("/etc/demo.sh", R_OK) == 0) {
		char *demo_argv[] = { "sh", "/etc/demo.sh", NULL };

		pid = fork();
		if (pid == 0) {
			execve("/bin/sh", demo_argv, sh_env);
			_exit(127);
		}
		while (waitpid(-1, &status, 0) != pid)
			;
	}

	/*
	 * With a framebuffer and buttons (the Mi 5), start Arcan with its
	 * terminal; the volume and power buttons drive it.  Should it exit,
	 * fall back to the button menu.
	 */
	if (access("/etc/arcan.sh", R_OK) == 0 &&
	    access("/dev/fb0", R_OK) == 0 &&
	    access("/dev/input/event0", R_OK) == 0) {
		static char autorun[512] = "autorun=uname -a; df -h / /tmp; ls /";
		char *ar_argv[] = { "sh", "/etc/arcan.sh", "term", autorun, NULL };
		int afd, n;

		/* /etc/arcan.autorun, if present, replaces the command. */
		if ((afd = open("/etc/arcan.autorun", O_RDONLY)) >= 0) {
			n = read(afd, autorun + 8, sizeof(autorun) - 9);
			if (n > 0) {
				autorun[8 + n] = 0;
				if (autorun[8 + n - 1] == '\n')
					autorun[8 + n - 1] = 0;
			}
			close(afd);
		}

		printf("init: starting Arcan\n");
		pid = fork();
		if (pid == 0) {
			execve("/bin/sh", ar_argv, sh_env);
			_exit(127);
		}
		while (waitpid(-1, &status, 0) != pid)
			;
		printf("init: Arcan exited (status %d)\n", status);
	}

	/*
	 * With buttons and no keyboard (the Mi 5), run the button menu;
	 * otherwise, and if it ever exits, a shell.
	 */
	if (access("/dev/input/event0", R_OK) == 0 &&
	    access("/bin/keymenu", R_OK) == 0) {
		char *km_argv[] = { "keymenu", NULL };

		pid = fork();
		if (pid == 0) {
			execve("/bin/keymenu", km_argv, sh_env);
			_exit(127);
		}
		while (waitpid(-1, &status, 0) != pid)
			;
	}

	/* Keep a shell running; reap whatever else exits. */
	for (;;) {
		pid = fork();
		if (pid == 0) {
			execve("/bin/sh", sh_argv, sh_env);
			printf("init: cannot exec /bin/sh (errno %d)\n", errno);
			_exit(127);
		}
		while (waitpid(-1, &status, 0) != pid)
			;
		printf("init: shell exited, starting another\n");
		sleep_ms(500);
	}
}
