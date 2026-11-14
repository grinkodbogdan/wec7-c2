/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * keymenu: drive the system from the phone's buttons until there is a
 * touchscreen or USB input.  Reads key events from an evdev device
 * (default /dev/input/event0):
 *
 *	volume up / down	move the selection
 *	power			run the selected command
 *	home			redraw the menu
 *
 * Commands come from /etc/keymenu.conf, one per line as
 * "label<TAB>shell command"; built-in ones are used without it.
 * Output stays on screen until the next key press.
 */
#include <sys/types.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <dev/misc/evdev/input.h>

#include <err.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define	MAXITEMS	32

struct item {
	char	*label;
	char	*cmd;
};

static struct item items[MAXITEMS];
static int nitems;

static const char *const builtin[][2] = {
	{ "System summary", "uname -a; sysctl hw.ncpu hw.physmem hw.machine; date" },
	{ "Processes", "ps ax | tail -25" },
	{ "Disks and filesystems", "df -h; echo; mount; echo; ls /dev/ufs*" },
	{ "Kernel messages (last 25)", "dmesg | tail -25" },
	{ "Mount persist (ext4, read-only)", "mount_ext2fs -o ro /dev/ufs0s11 /mnt && ls -l /mnt; umount /mnt" },
	{ "HAMMER2 test", "sh /etc/h2test.sh" },
	{ "libc self-test", "hello" },
	{ "Kernel self-test", "/sbin/init" },
	{ "Reboot", "reboot" },
};

static void
add(const char *label, const char *cmd)
{
	if (nitems >= MAXITEMS)
		return;
	items[nitems].label = strdup(label);
	items[nitems].cmd = strdup(cmd);
	++nitems;
}

static void
load(void)
{
	char line[512], *tab;
	FILE *f;
	size_t i;

	if ((f = fopen("/etc/keymenu.conf", "r")) != NULL) {
		while (fgets(line, sizeof(line), f) != NULL) {
			line[strcspn(line, "\n")] = 0;
			if (line[0] == '#' || (tab = strchr(line, '\t')) == NULL)
				continue;
			*tab++ = 0;
			add(line, tab);
		}
		fclose(f);
	}
	if (nitems == 0) {
		for (i = 0; i < sizeof(builtin) / sizeof(builtin[0]); ++i)
			add(builtin[i][0], builtin[i][1]);
	}
}

static void
draw(int sel)
{
	int i;

	printf("\033[2J\033[H");
	printf("DragonFly/arm64 on the Mi 5\n");
	printf("vol up/down: move   power: run   home: redraw\n\n");
	for (i = 0; i < nitems; ++i)
		printf("  %s %s\n", i == sel ? "==>" : "   ", items[i].label);
	fflush(stdout);
}

static void
run(int sel)
{
	pid_t pid;
	int status;

	printf("\033[2J\033[H# %s\n\n", items[sel].cmd);
	fflush(stdout);
	pid = fork();
	if (pid == 0) {
		execl("/bin/sh", "sh", "-c", items[sel].cmd, (char *)NULL);
		_exit(127);
	}
	if (pid > 0)
		waitpid(pid, &status, 0);
	printf("\n[done; any button returns to the menu]\n");
	fflush(stdout);
}

/* Next key press, as a KEY_* code. */
static int
next_key(int fd)
{
	struct input_event ev;

	for (;;) {
		if (read(fd, &ev, sizeof(ev)) != (ssize_t)sizeof(ev))
			err(1, "read");
		if (ev.type == EV_KEY && ev.value == 1)
			return (ev.code);
	}
}

int
main(int argc, char **argv)
{
	const char *dev = argc > 1 ? argv[1] : "/dev/input/event0";
	int fd, sel = 0, key, showing = 0;

	if ((fd = open(dev, O_RDONLY)) < 0)
		err(1, "%s", dev);
	load();
	draw(sel);
	for (;;) {
		key = next_key(fd);
		if (showing) {		/* any key leaves command output */
			showing = 0;
			draw(sel);
			continue;
		}
		switch (key) {
		case KEY_VOLUMEUP:
			sel = (sel + nitems - 1) % nitems;
			draw(sel);
			break;
		case KEY_VOLUMEDOWN:
			sel = (sel + 1) % nitems;
			draw(sel);
			break;
		case KEY_POWER:
			run(sel);
			showing = 1;
			break;
		case KEY_HOME:
			draw(sel);
			break;
		}
	}
}
