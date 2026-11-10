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
 * A very small shell for the DragonFly/arm64 test ramdisk.
 *
 * Not sh(1): no quoting, variables, pipes or scripts.  It splits a
 * line into words, understands "> file" and ">> file", and runs each
 * command in a child process so that redirection and ^C work the same
 * for the built-in commands as for programs on disk.
 */

#include "libc.h"

#define	MAXARGS		16

static void
cmd_help(void)
{
	puts("Built in:  cd DIR, exit, help");
	puts("Commands:  echo WORDS..    ls [DIR]       cat FILE..");
	puts("           mkdir DIR       rmdir DIR      rm FILE..");
	puts("           pwd             uname          uptime");
	puts("           sleep SECONDS   kill PID       spin (until ^C)");
	puts("           pi              reboot         poweroff");
	puts("           /path/to/program [ARGS..]   (e.g. /sbin/init)");
	puts("Redirect:  COMMAND > FILE, COMMAND >> FILE   (try /tmp)");
}

static int
cmd_ls(const char *dir)
{
	char buf[4096];
	struct dirent *dp;
	int fd, n, off;

	fd = open(dir, O_RDONLY);
	if (fd < 0) {
		printf("ls: %s: errno %d\n", dir, errno);
		return (1);
	}
	while ((n = getdents(fd, buf, sizeof(buf))) > 0) {
		for (off = 0; off < n; off += DIRENT_RECLEN(dp)) {
			dp = (struct dirent *)(buf + off);
			if (dp->d_ino == 0 || strcmp(dp->d_name, ".") == 0 ||
			    strcmp(dp->d_name, "..") == 0)
				continue;
			printf("%s%s\n", dp->d_name,
			    dp->d_type == DT_DIR ? "/" : "");
		}
	}
	close(fd);
	return (0);
}

static int
cmd_cat(const char *path)
{
	char buf[512];
	int fd, n;

	fd = open(path, O_RDONLY);
	if (fd < 0) {
		printf("cat: %s: errno %d\n", path, errno);
		return (1);
	}
	while ((n = read(fd, buf, sizeof(buf))) > 0)
		write(1, buf, n);
	close(fd);
	return (0);
}

/* Leibniz series: slow, but it is floating point all the way. */
static void
cmd_pi(void)
{
	double pi = 0.0, sign = 1.0;
	long i;

	for (i = 0; i < 2000000; ++i) {
		pi += sign * 4.0 / (double)(2 * i + 1);
		sign = -sign;
	}
	printf("pi is approximately %.6f\n", pi);
}

/* Runs in the child.  Returns the exit status. */
static int
run_command(int argc, char **argv)
{
	char buf[256];
	int i, rc = 0;

	if (strcmp(argv[0], "help") == 0) {
		cmd_help();
	} else if (strcmp(argv[0], "echo") == 0) {
		for (i = 1; i < argc; ++i)
			printf("%s%s", i > 1 ? " " : "", argv[i]);
		printf("\n");
	} else if (strcmp(argv[0], "ls") == 0) {
		rc = cmd_ls(argc > 1 ? argv[1] : ".");
	} else if (strcmp(argv[0], "cat") == 0) {
		for (i = 1; i < argc; ++i)
			rc |= cmd_cat(argv[i]);
	} else if (strcmp(argv[0], "mkdir") == 0 && argc > 1) {
		if ((rc = mkdir(argv[1], 0755)) != 0)
			printf("mkdir: %s: errno %d\n", argv[1], errno);
	} else if (strcmp(argv[0], "rmdir") == 0 && argc > 1) {
		if ((rc = rmdir(argv[1])) != 0)
			printf("rmdir: %s: errno %d\n", argv[1], errno);
	} else if (strcmp(argv[0], "rm") == 0) {
		for (i = 1; i < argc; ++i) {
			if (unlink(argv[i]) != 0) {
				printf("rm: %s: errno %d\n", argv[i], errno);
				rc = 1;
			}
		}
	} else if (strcmp(argv[0], "pwd") == 0) {
		puts(getcwd(buf, sizeof(buf)) ? buf : "?");
	} else if (strcmp(argv[0], "uname") == 0) {
		char rel[64], mach[32], model[64];

		sysctl_string(1, 1, buf, sizeof(buf));	/* kern.ostype */
		sysctl_string(1, 2, rel, sizeof(rel));	/* kern.osrelease */
		sysctl_string(6, 1, mach, sizeof(mach));/* hw.machine */
		sysctl_string(6, 2, model, sizeof(model)); /* hw.model */
		printf("%s %s %s (%s)\n", buf, rel, mach, model);
	} else if (strcmp(argv[0], "uptime") == 0) {
		long ms = uptime_ms();

		printf("up %ld.%03ld seconds\n", ms / 1000, ms % 1000);
	} else if (strcmp(argv[0], "sleep") == 0 && argc > 1) {
		sleep_ms(atol(argv[1]) * 1000);
	} else if (strcmp(argv[0], "kill") == 0 && argc > 1) {
		if ((rc = kill(atol(argv[1]), SIGTERM)) != 0)
			printf("kill: errno %d\n", errno);
	} else if (strcmp(argv[0], "spin") == 0) {
		puts("spinning; press ^C to stop");
		for (;;)
			;
	} else if (strcmp(argv[0], "pi") == 0) {
		cmd_pi();
	} else if (strcmp(argv[0], "reboot") == 0) {
		reboot(RB_AUTOBOOT);
	} else if (strcmp(argv[0], "poweroff") == 0) {
		reboot(RB_POWEROFF);
	} else if (strchr(argv[0], '/') != NULL) {
		execve(argv[0], argv, environ);
		printf("%s: cannot execute (errno %d)\n", argv[0], errno);
		rc = 127;
	} else {
		printf("%s: unknown command (try help)\n", argv[0]);
		rc = 127;
	}
	return (rc);
}

int
main(void)
{
	char line[256];
	char *argv[MAXARGS + 1];
	char *p, *redirect;
	int argc, append, status, fd, n;
	pid_t pid;

	/* ^C and ^Z are for the commands we run, not for the shell. */
	signal(SIGINT, SIG_IGN);
	signal(SIGTSTP, SIG_IGN);

	puts("\nDragonFly/arm64 test shell.  Type help for commands.");
	for (;;) {
		printf("# ");
		n = getline_fd(0, line, sizeof(line));
		if (n < 0)
			break;

		/* Split into words; pick out a redirection. */
		argc = 0;
		redirect = NULL;
		append = 0;
		for (p = line; *p != '\0'; ) {
			while (*p == ' ' || *p == '\t')
				*p++ = '\0';
			if (*p == '\0')
				break;
			if (*p == '>') {
				append = (p[1] == '>');
				*p = '\0';
				p += append ? 2 : 1;
				while (*p == ' ')
					++p;
				redirect = p;
			} else if (argc < MAXARGS) {
				argv[argc++] = p;
			}
			while (*p != '\0' && *p != ' ' && *p != '\t' &&
			    *p != '>')
				++p;
		}
		argv[argc] = NULL;
		if (argc == 0)
			continue;

		if (strcmp(argv[0], "exit") == 0)
			break;
		if (strcmp(argv[0], "cd") == 0) {
			if (chdir(argc > 1 ? argv[1] : "/") != 0)
				printf("cd: errno %d\n", errno);
			continue;
		}

		pid = fork();
		if (pid < 0) {
			printf("fork: errno %d\n", errno);
			continue;
		}
		if (pid == 0) {
			signal(SIGINT, SIG_DFL);
			signal(SIGTSTP, SIG_DFL);
			if (redirect != NULL && *redirect != '\0') {
				fd = open(redirect, O_WRONLY | O_CREAT |
				    (append ? O_APPEND : O_TRUNC), 0644);
				if (fd < 0) {
					printf("%s: cannot create (errno %d)\n",
					    redirect, errno);
					_exit(1);
				}
				dup2(fd, 1);
				close(fd);
			}
			_exit(run_command(argc, argv));
		}
		while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
			;
		if (WIFSIGNALED(status))
			printf("[killed by signal %d]\n", WTERMSIG(status));
		else if (WEXITSTATUS(status) != 0 &&
		    WEXITSTATUS(status) != 127)
			printf("[exit status %d]\n", WEXITSTATUS(status));
	}
	return (0);
}
