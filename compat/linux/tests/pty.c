/* pty.c - pseudo-terminals, as a terminal emulator uses them: posix_openpt,
 * grantpt/unlockpt/ptsname, the slave as a session's controlling terminal,
 * canonical input with echo, output with \n sent as \r\n, raw mode, window
 * size (and SIGWINCH), ^C as SIGINT to the foreground group, /dev/tty, a
 * shell running on the slave, and end of file once the slave side closes.
 * Prints "pty: ok" when everything passed. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

static int fails;
#define CHECK(c, what) do { if (c) printf("pty: %s ok\n", what); else { printf("pty: FAILED %s\n", what); fails++; } } while (0)

/* Everything read from the current master so far: a check looks for its
 * text anywhere in it (one read can bring several lines). */
static char acc[8192];
static size_t accn;

/* Read from the master until `want` shows up or 3 s pass; `buf` gets what
 * was read since the last reset. */
static int read_until(int m, char *buf, size_t cap, const char *want)
{
	for (int i = 0; i < 300 && !strstr(acc, want); i++) {
		struct pollfd p = { m, POLLIN, 0 };
		if (poll(&p, 1, 10) <= 0)
			continue;
		ssize_t n = read(m, acc + accn, sizeof acc - 1 - accn);
		if (n <= 0)
			break;
		accn += (size_t)n;
		acc[accn] = 0;
	}
	snprintf(buf, cap, "%s", acc);
	return strstr(acc, want) != NULL;
}

static volatile sig_atomic_t winch;
static void on_winch(int s) { (void)s; winch = 1; }

int main(void)
{
	setvbuf(stdout, NULL, _IONBF, 0);
	int m = posix_openpt(O_RDWR | O_NOCTTY);
	CHECK(m >= 0, "posix_openpt");
	CHECK(grantpt(m) == 0 && unlockpt(m) == 0, "grantpt/unlockpt");
	char *name = ptsname(m);
	CHECK(name && strncmp(name, "/dev/pts/", 9) == 0, "ptsname");
	char buf[512];

	/* A child: its own session, the slave as its controlling terminal. */
	pid_t pid = fork();
	if (pid == 0) {
		setsid();
		int s = open(name, O_RDWR);
		if (s < 0)
			_exit(10);
		dup2(s, 0), dup2(s, 1), dup2(s, 2);
		/* canonical: a line typed on the master, with echo */
		char line[64] = "";
		ssize_t n = read(0, line, sizeof line - 1);
		line[n > 0 ? n : 0] = 0;
		printf("got [%.*s]\n", (int)strcspn(line, "\n"), line);
		/* raw mode: bytes as they come, no echo */
		struct termios t;
		tcgetattr(0, &t);
		cfmakeraw(&t);
		tcsetattr(0, TCSANOW, &t);
		char c;
		if (read(0, &c, 1) == 1)
			printf("raw [%c]\r\n", c);
		/* window size, and SIGWINCH when the master changes it */
		signal(SIGWINCH, on_winch);
		struct winsize ws;
		ioctl(0, TIOCGWINSZ, &ws);
		printf("size %dx%d\r\n", ws.ws_col, ws.ws_row);
		for (int i = 0; i < 300 && !winch; i++)
			usleep(10000);
		ioctl(0, TIOCGWINSZ, &ws);
		printf("winch %d %dx%d\r\n", (int)winch, ws.ws_col, ws.ws_row);
		/* /dev/tty is this terminal */
		int tty = open("/dev/tty", O_RDWR);
		if (tty >= 0)
			write(tty, "via /dev/tty\r\n", 14);
		else
			printf("open /dev/tty: %s\r\n", strerror(errno));
		/* back to cooked; ^C from the master interrupts us */
		t.c_lflag |= ICANON | ECHO | ISIG;
		t.c_iflag |= ICRNL;
		t.c_oflag |= OPOST | ONLCR;
		tcsetattr(0, TCSANOW, &t);
		printf("waiting for ^C\n");
		pause();
		_exit(0);
	}
	CHECK(pid > 0, "fork");
	struct winsize ws = { .ws_row = 24, .ws_col = 80 };
	ioctl(m, TIOCSWINSZ, &ws);
	write(m, "hello\r", 6);
	CHECK(read_until(m, buf, sizeof buf, "got [hello]\r\n"), "canonical line, echo and \\n as \\r\\n");
	CHECK(strncmp(buf, "hello\r\n", 7) == 0, "echo");
	write(m, "x", 1);
	CHECK(read_until(m, buf, sizeof buf, "raw [x]"), "raw mode");
	CHECK(read_until(m, buf, sizeof buf, "size 80x24"), "window size");
	ws.ws_col = 100, ws.ws_row = 30;
	ioctl(m, TIOCSWINSZ, &ws);
	CHECK(read_until(m, buf, sizeof buf, "winch 1 100x30"), "SIGWINCH");
	CHECK(read_until(m, buf, sizeof buf, "via /dev/tty"), "/dev/tty");
	CHECK(read_until(m, buf, sizeof buf, "waiting for ^C"), "back to cooked");
	write(m, "\003", 1);
	int st = 0;
	for (int i = 0; i < 300 && waitpid(pid, &st, WNOHANG) == 0; i++)
		usleep(10000);
	CHECK(WIFSIGNALED(st) && WTERMSIG(st) == SIGINT, "^C is SIGINT");

	/* A shell on a terminal, as the phone's terminal app runs one. */
	accn = 0, acc[0] = 0;
	int m2 = posix_openpt(O_RDWR | O_NOCTTY);
	grantpt(m2), unlockpt(m2);
	char name2[64];
	snprintf(name2, sizeof name2, "%s", ptsname(m2));
	pid = fork();
	if (pid == 0) {
		setsid();
		int s = open(name2, O_RDWR);
		dup2(s, 0), dup2(s, 1), dup2(s, 2);
		close(m2);
		execl("/bin/sh", "sh", "-i", (char *)NULL);
		_exit(127);
	}
	write(m2, "echo shell-$((6*7))\r", 20);
	CHECK(read_until(m2, buf, sizeof buf, "shell-42"), "shell on a terminal");
	write(m2, "exit\r", 5);
	waitpid(pid, &st, 0);
	CHECK(WIFEXITED(st), "shell exit");
	/* every slave closed: the master reads end of file */
	ssize_t n = 0;
	for (int i = 0; i < 100; i++) {
		n = read(m2, buf, sizeof buf);
		if (n <= 0)
			break;
	}
	CHECK(n <= 0, "end of file after hang-up");
	printf(fails ? "pty: FAILED (%d)\n" : "pty: ok\n", fails);
	return fails != 0;
}
