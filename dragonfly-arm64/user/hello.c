/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * Smoke test for the real C library on arm64: each line exercises a
 * piece of libc that the port had to supply (startup, TLS, syscall
 * stubs and errno, setjmp, signals, ucontext, stdio's floating point,
 * malloc, threads).
 */
#include <sys/types.h>
#include <sys/wait.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <ucontext.h>
#include <unistd.h>

static int failures;
static __thread int tls_counter = 42;
static __thread char tls_buf[64];

static void
check(const char *what, int ok)
{
	printf("  %-6s %s\n", ok ? "ok" : "FAILED", what);
	if (!ok)
		failures++;
}

static jmp_buf jb;
static volatile sig_atomic_t got;

static void
on_alarm(int sig)
{
	got = sig;
}

static ucontext_t uc_main, uc_func;
static char uc_stack[64 * 1024];
static int uc_ran;

static void
uc_body(int a, int b)
{
	uc_ran = a + b;
}

static void *
thread_main(void *arg)
{
	long n = (long)arg;

	tls_counter = (int)n;		/* each thread its own copy */
	snprintf(tls_buf, sizeof(tls_buf), "thread %ld", n);
	usleep(1000 * (int)n);
	return ((void *)(long)(tls_counter * 10 + (int)strlen(tls_buf)));
}

int
main(int argc, char **argv)
{
	char buf[128];
	pthread_t th[4];
	void *ret;
	char *p;
	int fd, i, ok, pfd[2], status;
	pid_t pid;
	long double ld;

	printf("hello from DragonFly libc on %s, argc %d, argv[0] %s\n",
	    "aarch64", argc, argv[0]);

	check("initialized TLS", tls_counter == 42);
	strcpy(tls_buf, "main");

	errno = 0;
	fd = open("/nonexistent", O_RDONLY);
	check("failed syscall sets errno", fd == -1 && errno == ENOENT);

	snprintf(buf, sizeof(buf), "%.3f %g %Lf %e", M_PI, 1e-10,
	    (long double)2.5, sqrt(2.0));
	check("printf floating point", strcmp(buf,
	    "3.142 1e-10 2.500000 1.414214e+00") == 0);
	printf("         (%s)\n", buf);

	ld = strtold("1.000000000000000000000000000001", NULL);
	check("long double is quad precision", ld > 1.0L && ld - 1.0L < 1e-29L);

	p = malloc(1 << 20);
	memset(p, 'x', 1 << 20);
	p = realloc(p, 4 << 20);
	check("malloc/realloc", p != NULL && p[12345] == 'x');
	free(p);

	if (setjmp(jb) == 0)
		longjmp(jb, 7);
	check("setjmp/longjmp", 1);

	signal(SIGALRM, on_alarm);
	alarm(1);
	pause();
	check("signal handler via sigaction", got == SIGALRM);

	getcontext(&uc_func);
	uc_func.uc_stack.ss_sp = uc_stack;
	uc_func.uc_stack.ss_size = sizeof(uc_stack);
	uc_func.uc_link = &uc_main;
	makecontext(&uc_func, (void (*)(void))uc_body, 2, 40, 2);
	swapcontext(&uc_main, &uc_func);
	check("makecontext/swapcontext", uc_ran == 42);

	ok = pipe(pfd) == 0;
	pid = fork();
	if (pid == 0) {
		close(pfd[0]);
		write(pfd[1], "child", 5);
		_exit(3);
	}
	close(pfd[1]);
	memset(buf, 0, sizeof(buf));
	read(pfd[0], buf, sizeof(buf));
	waitpid(pid, &status, 0);
	check("pipe, fork, waitpid", ok && strcmp(buf, "child") == 0 &&
	    WIFEXITED(status) && WEXITSTATUS(status) == 3);

	ok = 1;
	for (i = 0; i < 4; i++)
		ok &= pthread_create(&th[i], NULL, thread_main,
		    (void *)(long)(i + 1)) == 0;
	for (i = 0; i < 4; i++) {
		ok &= pthread_join(th[i], &ret) == 0;
		ok &= (long)ret == (i + 1) * 10 + 8;
	}
	check("threads with their own TLS", ok);
	check("main thread TLS untouched", tls_counter == 42 &&
	    strcmp(tls_buf, "main") == 0);

	printf("%s\n", failures ? "SOME CHECKS FAILED" : "libc: all checks passed");
	return (failures != 0);
}
