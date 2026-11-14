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
 * A minimal C library for DragonFly/arm64 test programs.
 *
 * This is NOT DragonFly's libc.  The real one has not been ported
 * (it needs a cross toolchain and per-CPU assembly for system calls,
 * setjmp, thread-local storage and the runtime linker).  This is a
 * few hundred lines of freestanding code: enough system call wrappers,
 * string functions, printf and malloc to write small programs that
 * exercise the kernel.  Structure layouts and constants are copied
 * from the kernel's headers and must match them.
 */

#ifndef _MINI_LIBC_H_
#define	_MINI_LIBC_H_

typedef unsigned long	size_t;
typedef long		ssize_t;
typedef long		off_t;
typedef int		pid_t;
typedef __builtin_va_list va_list;
#define	va_start(ap, last)	__builtin_va_start(ap, last)
#define	va_arg(ap, type)	__builtin_va_arg(ap, type)
#define	va_end(ap)		__builtin_va_end(ap)
#define	NULL			((void *)0)

extern int errno;
extern char **environ;

#define	ENOENT		2
#define	EINTR		4
#define	ECHILD		10
#define	EEXIST		17

#define	O_RDONLY	0x0000
#define	O_WRONLY	0x0001
#define	O_RDWR		0x0002
#define	O_APPEND	0x0008
#define	O_CREAT		0x0200
#define	O_TRUNC		0x0400

#define	SIGINT		2
#define	SIGKILL		9
#define	SIGSEGV		11
#define	SIGTERM		15
#define	SIGTSTP		18
#define	SIGCHLD		20
#define	SIGTTIN		21
#define	SIGTTOU		22
#define	SIGUSR1		30
#define	SIG_DFL		((void (*)(int))0)
#define	SIG_IGN		((void (*)(int))1)

#define	RB_AUTOBOOT	0
#define	RB_POWEROFF	0x4000

#define	DT_DIR		4

#define	WIFEXITED(s)	(((s) & 0x7f) == 0)
#define	WEXITSTATUS(s)	(((s) >> 8) & 0xff)
#define	WIFSIGNALED(s)	(((s) & 0x7f) != 0 && ((s) & 0x7f) != 0x7f)
#define	WTERMSIG(s)	((s) & 0x7f)

/* <sys/dirent.h> */
struct dirent {
	unsigned long	d_ino;
	unsigned short	d_namlen;
	unsigned char	d_type;
	unsigned char	d_unused1;
	unsigned int	d_unused2;
	char		d_name[256];
};
#define	DIRENT_RECLEN(dp)	((16 + (dp)->d_namlen + 1 + 7) & ~7)

struct timespec {
	long	tv_sec;
	long	tv_nsec;
};

/* raw system call; returns -1 and sets errno on failure */
long	syscall(long num, ...);

void	_exit(int status) __attribute__((noreturn));
void	exit(int status) __attribute__((noreturn));
pid_t	fork(void);
pid_t	getpid(void);
pid_t	getppid(void);
pid_t	waitpid(pid_t pid, int *status, int options);
int	execve(const char *path, char *const argv[], char *const envp[]);
int	kill(pid_t pid, int sig);
void	(*signal(int sig, void (*handler)(int)))(int);
int	open(const char *path, int flags, ...);
int	close(int fd);
ssize_t	read(int fd, void *buf, size_t len);
ssize_t	write(int fd, const void *buf, size_t len);
int	dup2(int from, int to);
int	unlink(const char *path);
int	access(const char *path, int mode);
#define	R_OK	0x04
int	mkdir(const char *path, int mode);
int	rmdir(const char *path);
int	chdir(const char *path);
char	*getcwd(char *buf, size_t size);
int	getdents(int fd, char *buf, size_t len);
int	mount(const char *type, const char *dir, int flags, void *data);
int	reboot(int howto);
int	setsid(void);
int	ioctl(int fd, unsigned long cmd, void *arg);
int	sleep_ms(long ms);
long	uptime_ms(void);
int	sysctl_string(int top, int second, char *buf, size_t size);
int	sysctl_int(int top, int second, int *val);
int	sysctlbyname(const char *name, void *oldp, size_t *oldlenp);
void	*mmap_anon(size_t len);

size_t	strlen(const char *s);
int	strcmp(const char *a, const char *b);
int	strncmp(const char *a, const char *b, size_t n);
char	*strcpy(char *dst, const char *src);
char	*strchr(const char *s, int c);
void	*memcpy(void *dst, const void *src, size_t n);
void	*memset(void *dst, int c, size_t n);
long	atol(const char *s);

void	*malloc(size_t size);
void	free(void *ptr);

int	printf(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
int	dprintf(int fd, const char *fmt, ...)
	    __attribute__((format(printf, 2, 3)));
int	puts(const char *s);
int	getline_fd(int fd, char *buf, size_t size);

#endif /* !_MINI_LIBC_H_ */
