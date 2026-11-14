/*
 * Named POSIX semaphores across processes (Arcan's shmif depends on them).
 */
#include <sys/types.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <semaphore.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

static long
ms(void)
{
	struct timespec ts;

	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
}

int
main(void)
{
	sem_t *a, *b;
	long t0;
	int i, st;

	sem_unlink("/semtest_a");
	sem_unlink("/semtest_b");
	a = sem_open("/semtest_a", O_CREAT, 0600, 0);
	b = sem_open("/semtest_b", O_CREAT, 0600, 0);
	if (a == SEM_FAILED || b == SEM_FAILED) {
		perror("sem_open");
		return (1);
	}
	if (fork() == 0) {
		sem_t *ca = sem_open("/semtest_a", 0), *cb = sem_open("/semtest_b", 0);
		for (i = 0; i < 1000; i++) {	/* ping-pong */
			sem_wait(ca);
			sem_post(cb);
		}
		_exit(0);
	}
	t0 = ms();
	for (i = 0; i < 1000; i++) {
		sem_post(a);
		sem_wait(b);
	}
	printf("sem: 1000 cross-process round trips in %ld ms\n", ms() - t0);
	wait(&st);
	sem_unlink("/semtest_a");
	sem_unlink("/semtest_b");
	return (0);
}
