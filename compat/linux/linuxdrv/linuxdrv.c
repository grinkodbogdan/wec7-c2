/*
 * linuxdrv - Linux's own device drivers on Flare: the Linux kernel as a library
 * (LKL) in an ordinary Linux process, for the hardware that is hard or expensive
 * to drive natively (eMMC, USB, I2C touch screens, PMIC regulators, ...).
 *
 *   linuxdrv BOARD.dtb [Linux kernel arguments...]
 *
 * The approach is QRT's (github.com/grinkodbogdan/qrt): its /bin/linuxdrv runs LKL
 * as a user program on x86, and its ARM phone port (Xiaomi Mi A1) runs Linux's
 * Qualcomm drivers from the boot loader's device tree, with patches to LKL for
 * device trees, the GIC, non-coherent DMA and secure monitor calls
 * (compat/linux/linuxdrv/lkl/qrt.patch).  Flare keeps drivers isolated, so here
 * Linux runs in a user process like QRT's x86 host, with the ARM port's device
 * tree support, and gets its hardware from Flare's kernel through /dev/flarehw:
 * exactly the devices, interrupts and firmware calls the boot composition names
 * in [linux] hardware / smc (kernel/src/linux/flarehw.rs).
 *
 * The host operations LKL asks for, on Flare:
 *
 *   threads, semaphores, mutexes, TLS   pthreads
 *   timers                              a thread per timer, condition variables
 *   RAM (LKL's MMU mode)                one physically contiguous block from
 *                                       /dev/flarehw, mapped at LKL's window, so a
 *                                       driver's DMA address is real (QRT's "Argon")
 *   coherent DMA memory                 a second block, mapped non-cacheable
 *   device registers                    ioremap maps the granted register window
 *   interrupts                          Linux's GIC driver (qrt-gic) masks and
 *                                       unmasks through the host (irq_ctl); a thread
 *                                       reads fired interrupts from /dev/flarehw and
 *                                       raises them in Linux (lkl_trigger_irq)
 *   SMC calls (Qualcomm SCM)            through the host (smc), allow-listed by Flare
 *
 * SPDX-License-Identifier: GPL-2.0 (it links Linux)
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/auxv.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#include <zlib.h>
#include <lkl/asm/host_ops.h>

/* ---- lkl.o ---- */
int lkl_init(struct lkl_host_operations *ops);
int lkl_start_kernel(const char *cmd, ...);
long lkl_syscall(long no, long *params);
int lkl_trigger_irq(int irq);
extern void *lkl_qrt_fdt;
extern unsigned long lkl_qrt_dma_base, lkl_qrt_dma_size, lkl_qrt_ram_phys;

#define GIC_VIRQ_BASE 1024 /* qrt-gic: hwirq n is Linux irq 1024 + n */

/* ---- /dev/flarehw (kernel/src/linux/flarehw.rs) ---- */
#define FLAREHW_INFO      0x4600
#define FLAREHW_DMA_ALLOC 0x4601
#define FLAREHW_IRQ       0x4602
#define FLAREHW_SMC       0x4603
#define FLAREHW_NOCACHE   (1ull << 62)

struct hw_info {
	uint32_t index, nregs, nirqs;
	char name[32];
	uint64_t regs[8][2];
	uint32_t irqs[8];
};

static int hw;

/* Seconds since boot (Flare's clock), for the boot timeline. */
static double since_boot(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec + ts.tv_nsec / 1e9;
}

static int dma_alloc(uint64_t size, uint64_t *phys)
{
	uint64_t a[2] = { size, 0 };
	if (ioctl(hw, FLAREHW_DMA_ALLOC, a))
		return -1;
	*phys = a[1];
	return 0;
}

/* ---- the granted register windows, mapped on first ioremap ---- */
#define MAXWIN 512
static struct { uint64_t base, size; char *va; char name[32]; } win[MAXWIN];
static int nwin;
static pthread_mutex_t win_lock = PTHREAD_MUTEX_INITIALIZER;

/* RAM and the coherent pool: physical base, size, where this process sees them */
static uint64_t ram_phys, ram_size;
static uint64_t pool_phys, pool_size;
static char *pool_va;

static void *h_ioremap(long addr, int size)
{
	uint64_t a = (uint64_t)addr;
	if (a >= pool_phys && a + (uint64_t)size <= pool_phys + pool_size)
		return pool_va + (a - pool_phys);
	pthread_mutex_lock(&win_lock);
	for (int i = 0; i < nwin; i++) {
		if (a < win[i].base || a + (uint64_t)size > win[i].base + win[i].size)
			continue;
		if (!win[i].va) {
			uint64_t b = win[i].base & ~0xfffull;
			uint64_t len = ((win[i].base + win[i].size + 0xfff) & ~0xfffull) - b;
			void *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, hw, (off_t)b);
			if (p == MAP_FAILED) {
				fprintf(stderr, "linuxdrv: cannot map %s (%#llx): %s\n", win[i].name,
					(unsigned long long)b, strerror(errno));
				break;
			}
			win[i].va = (char *)p + (win[i].base - b);
		}
		pthread_mutex_unlock(&win_lock);
		return win[i].va + (a - win[i].base);
	}
	pthread_mutex_unlock(&win_lock);
	fprintf(stderr, "linuxdrv: ioremap(%#lx, %d): not hardware this host was given\n", addr, size);
	return NULL;
}

static int h_iomem_access(const volatile void *addr, void *val, int size, int write)
{
	if (!addr)
		return -1;
	switch (size) {
	case 1: if (write) *(volatile uint8_t *)addr = *(uint8_t *)val; else *(uint8_t *)val = *(volatile uint8_t *)addr; break;
	case 2: if (write) *(volatile uint16_t *)addr = *(uint16_t *)val; else *(uint16_t *)val = *(volatile uint16_t *)addr; break;
	case 4: if (write) *(volatile uint32_t *)addr = *(uint32_t *)val; else *(uint32_t *)val = *(volatile uint32_t *)addr; break;
	case 8: if (write) *(volatile uint64_t *)addr = *(uint64_t *)val; else *(uint64_t *)val = *(volatile uint64_t *)addr; break;
	default: return -1;
	}
	return 0;
}

/* ---- interrupts ---- */
static long h_irq_ctl(int hwirq, int op, int arg)
{
	uint32_t a[3] = { (uint32_t)hwirq, (uint32_t)op, (uint32_t)arg };
	long r = ioctl(hw, FLAREHW_IRQ, a);
	if (r < 0 && op != 0)
		fprintf(stderr, "linuxdrv: interrupt %d (op %d) refused: %s\n", hwirq, op, strerror(errno));
	return r < 0 ? 0 : r;
}

static void *irq_thread(void *p)
{
	(void)p;
	for (;;) {
		uint32_t ids[64];
		ssize_t n = read(hw, ids, sizeof ids);
		if (n <= 0) {
			if (n < 0 && errno != EINTR && errno != EAGAIN)
				usleep(1000);
			continue;
		}
		for (ssize_t i = 0; i < n / 4; i++)
			lkl_trigger_irq(GIC_VIRQ_BASE + (int)ids[i]);
	}
	return NULL;
}

/* ---- secure monitor calls ---- */
static void h_smc(unsigned long *r)
{
	if (ioctl(hw, FLAREHW_SMC, r)) {
		fprintf(stderr, "linuxdrv: SMC %#lx refused: %s\n", r[0], strerror(errno));
		r[0] = (unsigned long)-1; /* SMCCC_RET_NOT_SUPPORTED */
	}
}

/* ---- the rest of LKL's host: pthreads, timers, memory ---- */
/* Linux's console. Everything goes into a ring kept outside Linux (a
 * driver that hangs Linux cannot keep it from being shown: the watchdog
 * below prints its tail); only lines that look like trouble go to Flare's
 * console as they come. */
#define KRING (512 << 10)
static char kring[KRING];
static unsigned kring_at;
static char kline[512];
static unsigned kline_len;
static pthread_mutex_t kring_lock = PTHREAD_MUTEX_INITIALIZER;

static int trouble(const char *l)
{
	static const char *const w[] = { "rror", "fail", "Fail", "nable to", "timeout", "timed out", "WARNING", "BUG",
					 "Oops", "panic", "-110", "-22", NULL };
	for (int i = 0; w[i]; i++)
		if (strstr(l, w[i]))
			return 1;
	return 0;
}

static void h_print(const char *s, int len)
{
	pthread_mutex_lock(&kring_lock);
	for (int i = 0; i < len; i++) {
		kring[kring_at++ % KRING] = s[i];
		if (s[i] == '\n' || kline_len == sizeof kline - 1) {
			kline[kline_len] = 0;
			if (trouble(kline))
				fprintf(stderr, "linux: %s\n", kline);
			kline_len = 0;
		} else {
			kline[kline_len++] = s[i];
		}
	}
	pthread_mutex_unlock(&kring_lock);
}

/* the last `lines` lines Linux printed */
static void kring_tail(int lines)
{
	pthread_mutex_lock(&kring_lock);
	unsigned end = kring_at, start = end > KRING ? end - KRING : 0, at = end;
	int n = 0;
	while (at > start && n <= lines)
		if (kring[--at % KRING] == '\n')
			n++;
	static char out[KRING + 1];
	unsigned k = 0;
	for (unsigned i = at; i < end; i++)
		out[k++] = kring[i % KRING];
	out[k] = 0;
	pthread_mutex_unlock(&kring_lock);
	fprintf(stderr, "%s\n", out);
}

static long l_sys(long nr, long a, long b, long c, long d, long e);

/* Linux's slowest start-up steps (initcall_debug: "initcall F+0x.. returned
 * 0 after N usecs"), from the ring */
static void slowest_initcalls(void)
{
	struct { long us; char name[64]; } top[8] = { 0 };
	/* Linux's own log (syslog: read all) */
	static char klog[1 << 20];
	long kn = l_sys(116, 3, (long)klog, sizeof klog, 0, 0);
	if (kn <= 0)
		return;
	static char line[256];
	unsigned k = 0;
	for (long i = 0; i <= kn; i++) {
		char ch = i < kn ? klog[i] : '\n';
		if (ch != '\n' && k < sizeof line - 1) {
			line[k++] = ch;
			continue;
		}
		line[k] = 0;
		k = 0;
		char *a = strstr(line, "initcall "), *b = strstr(line, " after "), *u = strstr(line, " usecs");
		if (!a || !b || !u || !strstr(line, "returned"))
			continue;
		long us = atol(b + 7);
		int m = 0;
		for (int j = 1; j < 8; j++)
			if (top[j].us < top[m].us)
				m = j;
		if (us <= top[m].us)
			continue;
		top[m].us = us;
		char *nm = a + 9, *e = strpbrk(nm, "+ ");
		int len = e ? (int)(e - nm) : 40;
		snprintf(top[m].name, sizeof top[m].name, "%.*s", len > 60 ? 60 : len, nm);
	}
	for (int i = 0; i < 8; i++)
		for (int j = i + 1; j < 8; j++)
			if (top[j].us > top[i].us) {
				__typeof__(top[0]) t = top[i];
				top[i] = top[j];
				top[j] = t;
			}
	if (!top[0].us)
		return;
	printf("linuxdrv: Linux's slowest start-up steps:");
	for (int i = 0; i < 5 && top[i].us; i++)
		printf("%s %s %ld ms", i ? "," : "", top[i].name, top[i].us / 1000);
	printf("\n");
}

static volatile int devices_ready;

/* Linux not done probing after 30 s (then 60 s): what it said last */
static void *watchdog(void *arg)
{
	(void)arg;
	for (int round = 1; round <= 2; round++) {
		for (int t = 0; t < 300 && !devices_ready; t++)
			usleep(100000);
		if (devices_ready)
			return NULL;
		fprintf(stderr, "linuxdrv: [%.1f s] Linux has not finished bringing up its devices; its last messages:\n",
			since_boot());
		kring_tail(18);
	}
	return NULL;
}
/* liblkl's helpers, which the kernel library calls (normally from liblkl). */
int lkl_printf(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	int n = vfprintf(stderr, fmt, ap);
	va_end(ap);
	return n;
}

void lkl_bug(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	fprintf(stderr, "linuxdrv: Linux bug: ");
	vfprintf(stderr, fmt, ap);
	va_end(ap);
	fflush(stderr);
	abort();
}

static void h_panic(void) { fprintf(stderr, "linuxdrv: Linux panicked\n"); abort(); }

struct lkl_sem { pthread_mutex_t m; pthread_cond_t c; int count; };
struct lkl_mutex { pthread_mutex_t m; };
struct lkl_tls_key { pthread_key_t k; };

static struct lkl_sem *sem_alloc(int count)
{
	struct lkl_sem *s = calloc(1, sizeof *s);
	pthread_mutex_init(&s->m, NULL);
	pthread_cond_init(&s->c, NULL);
	s->count = count;
	return s;
}
static void sem_free(struct lkl_sem *s) { pthread_cond_destroy(&s->c); pthread_mutex_destroy(&s->m); free(s); }
static void sem_up(struct lkl_sem *s)
{
	pthread_mutex_lock(&s->m);
	if (++s->count > 0)
		pthread_cond_signal(&s->c);
	pthread_mutex_unlock(&s->m);
}
static void sem_down(struct lkl_sem *s)
{
	pthread_mutex_lock(&s->m);
	while (s->count <= 0)
		pthread_cond_wait(&s->c, &s->m);
	s->count--;
	pthread_mutex_unlock(&s->m);
}
static struct lkl_mutex *mutex_alloc(int recursive)
{
	struct lkl_mutex *m = calloc(1, sizeof *m);
	pthread_mutexattr_t a;
	pthread_mutexattr_init(&a);
	pthread_mutexattr_settype(&a, recursive ? PTHREAD_MUTEX_RECURSIVE : PTHREAD_MUTEX_ERRORCHECK);
	pthread_mutex_init(&m->m, &a);
	pthread_mutexattr_destroy(&a);
	return m;
}
static void mutex_free(struct lkl_mutex *m) { pthread_mutex_destroy(&m->m); free(m); }
static void mutex_lock(struct lkl_mutex *m) { pthread_mutex_lock(&m->m); }
static void mutex_unlock(struct lkl_mutex *m) { pthread_mutex_unlock(&m->m); }

struct thread_arg { void (*fn)(void *); void *arg; };
static void *thread_start(void *p)
{
	struct thread_arg a = *(struct thread_arg *)p;
	free(p);
	a.fn(a.arg);
	return NULL;
}
static lkl_thread_t thread_create(void (*fn)(void *), void *arg)
{
	pthread_t t;
	pthread_attr_t at;
	struct thread_arg *a = malloc(sizeof *a);
	a->fn = fn;
	a->arg = arg;
	pthread_attr_init(&at);
	pthread_attr_setstacksize(&at, 256 * 1024);
	int e = pthread_create(&t, &at, thread_start, a);
	pthread_attr_destroy(&at);
	if (e) {
		free(a);
		return 0;
	}
	return (lkl_thread_t)t;
}
static void thread_detach(void) { pthread_detach(pthread_self()); }
static void thread_exit(void) { pthread_exit(NULL); }
static int thread_join(lkl_thread_t t) { return pthread_join((pthread_t)t, NULL) ? -1 : 0; }
static lkl_thread_t thread_self(void) { return (lkl_thread_t)pthread_self(); }
static int thread_equal(lkl_thread_t a, lkl_thread_t b) { return pthread_equal((pthread_t)a, (pthread_t)b); }
static void *thread_stack(unsigned long *size) { (void)size; return NULL; }
static struct lkl_tls_key *tls_alloc(void (*destructor)(void *))
{
	struct lkl_tls_key *k = malloc(sizeof *k);
	if (pthread_key_create(&k->k, destructor)) {
		free(k);
		return NULL;
	}
	return k;
}
static void tls_free(struct lkl_tls_key *k) { pthread_key_delete(k->k); free(k); }
static int tls_set(struct lkl_tls_key *k, void *d) { return pthread_setspecific(k->k, d) ? -1 : 0; }
static void *tls_get(struct lkl_tls_key *k) { return pthread_getspecific(k->k); }

static unsigned long long time_ns(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (unsigned long long)ts.tv_sec * 1000000000ull + (unsigned long long)ts.tv_nsec;
}

/* a thread per timer (QRT's linuxdrv does the same) */
struct qtimer { void (*fn)(void); pthread_t th; pthread_mutex_t m; pthread_cond_t c; unsigned long long due; int armed, quit; };
static void *timer_thread(void *p)
{
	struct qtimer *t = p;
	pthread_mutex_lock(&t->m);
	while (!t->quit) {
		if (!t->armed) {
			pthread_cond_wait(&t->c, &t->m);
			continue;
		}
		unsigned long long now = time_ns();
		if (now < t->due) {
			struct timespec ts;
			clock_gettime(CLOCK_MONOTONIC, &ts);
			unsigned long long d = t->due - now;
			ts.tv_sec += (time_t)(d / 1000000000ull);
			ts.tv_nsec += (long)(d % 1000000000ull);
			if (ts.tv_nsec >= 1000000000L) {
				ts.tv_sec++;
				ts.tv_nsec -= 1000000000L;
			}
			pthread_cond_timedwait(&t->c, &t->m, &ts);
			continue;
		}
		t->armed = 0;
		pthread_mutex_unlock(&t->m);
		t->fn();
		pthread_mutex_lock(&t->m);
	}
	pthread_mutex_unlock(&t->m);
	return NULL;
}
static void *timer_alloc(void (*fn)(void))
{
	struct qtimer *t = calloc(1, sizeof *t);
	pthread_condattr_t ca;
	t->fn = fn;
	pthread_mutex_init(&t->m, NULL);
	pthread_condattr_init(&ca);
	pthread_condattr_setclock(&ca, CLOCK_MONOTONIC);
	pthread_cond_init(&t->c, &ca);
	if (pthread_create(&t->th, NULL, timer_thread, t)) {
		free(t);
		return NULL;
	}
	return t;
}
static int timer_set_oneshot(void *p, unsigned long ns)
{
	struct qtimer *t = p;
	pthread_mutex_lock(&t->m);
	t->due = time_ns() + ns;
	t->armed = 1;
	pthread_cond_signal(&t->c);
	pthread_mutex_unlock(&t->m);
	return 0;
}
static void timer_free(void *p)
{
	struct qtimer *t = p;
	pthread_mutex_lock(&t->m);
	t->quit = 1;
	pthread_cond_signal(&t->c);
	pthread_mutex_unlock(&t->m);
	if (!pthread_equal(pthread_self(), t->th))
		pthread_join(t->th, NULL);
	else
		pthread_detach(t->th);
}

static void *mem_alloc(unsigned long n) { return malloc(n); }
static void *h_memcpy(void *d, const void *s, unsigned long n) { return memcpy(d, s, n); }
static void *h_memset(void *d, int c, unsigned long n) { return memset(d, c, n); }
static void *h_memmove(void *d, const void *s, unsigned long n) { return memmove(d, s, n); }

static void h_jmp_buf_set(struct lkl_jmp_buf *jmpb, void (*f)(void))
{
	if (!setjmp(*((jmp_buf *)jmpb->buf)))
		f();
}
static void h_jmp_buf_longjmp(struct lkl_jmp_buf *jmpb, int val) { longjmp(*((jmp_buf *)jmpb->buf), val); }

static int prot_of(enum lkl_prot p)
{
	return (p & LKL_PROT_READ ? PROT_READ : 0) | (p & LKL_PROT_WRITE ? PROT_WRITE : 0) |
	       (p & LKL_PROT_EXEC ? PROT_EXEC : 0);
}

/* LKL's MMU mode: "physical memory" is one contiguous block of real RAM */
static void h_shmem_init(unsigned long size)
{
	if (dma_alloc(size, &ram_phys)) {
		fprintf(stderr, "linuxdrv: no %lu MiB of contiguous RAM for Linux\n", size >> 20);
		abort();
	}
	ram_size = size;
	lkl_qrt_ram_phys = ram_phys;
}
static void *h_shmem_mmap(void *addr, unsigned long pg_off, unsigned long size, enum lkl_prot prot)
{
	uint64_t off = ram_phys + pg_off;
	if ((int)prot & 8 /* LKL_PROT_NOCACHE */)
		off |= FLAREHW_NOCACHE;
	void *p = mmap(addr, size, prot_of(prot), MAP_SHARED | (addr ? MAP_FIXED : 0), hw, (off_t)off);
	return p == MAP_FAILED ? NULL : p;
}
static void *h_mmap(void *addr, unsigned long size, enum lkl_prot prot)
{
	void *p = mmap(addr, size, prot_of(prot), MAP_PRIVATE | MAP_ANONYMOUS | (addr ? MAP_FIXED : 0), -1, 0);
	return p == MAP_FAILED ? NULL : p;
}
static int h_munmap(void *addr, unsigned long size) { return munmap(addr, size); }

static struct lkl_host_operations ops = {
	.print = h_print,
	.panic = h_panic,
	.sem_alloc = sem_alloc,
	.sem_free = sem_free,
	.sem_up = sem_up,
	.sem_down = sem_down,
	.mutex_alloc = mutex_alloc,
	.mutex_free = mutex_free,
	.mutex_lock = mutex_lock,
	.mutex_unlock = mutex_unlock,
	.thread_create = thread_create,
	.thread_detach = thread_detach,
	.thread_exit = thread_exit,
	.thread_join = thread_join,
	.thread_self = thread_self,
	.thread_equal = thread_equal,
	.thread_stack = thread_stack,
	.tls_alloc = tls_alloc,
	.tls_free = tls_free,
	.tls_set = tls_set,
	.tls_get = tls_get,
	.mem_alloc = mem_alloc,
	.mem_free = free,
	.time = time_ns,
	.timer_alloc = timer_alloc,
	.timer_set_oneshot = timer_set_oneshot,
	.timer_free = timer_free,
	.ioremap = h_ioremap,
	.iomem_access = h_iomem_access,
	.jmp_buf_set = h_jmp_buf_set,
	.jmp_buf_longjmp = h_jmp_buf_longjmp,
	.memcpy = h_memcpy,
	.memset = h_memset,
	.memmove = h_memmove,
	.mmap = h_mmap,
	.munmap = h_munmap,
	.shmem_init = h_shmem_init,
	.shmem_mmap = h_shmem_mmap,
	.irq_ctl = h_irq_ctl,
	.smc = h_smc,
};

/* ---- Linux's system calls, as the driver host makes them (asm-generic numbers) ---- */
#define NR_IOCTL    29
#define NR_MKDIRAT  34
#define NR_MOUNT    40
#define NR_OPENAT   56
#define NR_CLOSE    57
#define NR_GETDENTS 61
#define NR_READ     63
#define NR_PREAD64  67
#define NR_WRITE    64
#define L_BLKFLSBUF 0x1261 /* drop the device's cached blocks */
#define AT_FDCWD    (-100)
static long l_sys(long nr, long a, long b, long c, long d, long e)
{
	long p[6] = { a, b, c, d, e, 0 };
	return lkl_syscall(nr, p);
}
static long l_sys6(long nr, long a, long b, long c, long d, long e, long f)
{
	long p[6] = { a, b, c, d, e, f };
	return lkl_syscall(nr, p);
}
static long l_open(const char *path, int flags) { return l_sys(NR_OPENAT, AT_FDCWD, (long)path, flags, 0, 0); }
static long l_read(int fd, void *b, long n) { return l_sys(NR_READ, fd, (long)b, n, 0, 0); }
static long l_close(int fd) { return l_sys(NR_CLOSE, fd, 0, 0, 0, 0); }

/* every name in a Linux directory, space-separated */
static void l_list(const char *dir, char *out, int cap)
{
	char buf[2048];
	int at = 0;
	out[0] = 0;
	long fd = l_open(dir, 0x10000 /* O_DIRECTORY */);
	if (fd < 0)
		return;
	long n;
	while ((n = l_sys(NR_GETDENTS, fd, (long)buf, sizeof buf, 0, 0)) > 0) {
		for (long o = 0; o < n;) {
			unsigned short reclen = *(unsigned short *)(buf + o + 16);
			const char *name = buf + o + 19;
			if (name[0] != '.' && at + (int)strlen(name) + 2 < cap)
				at += snprintf(out + at, (size_t)(cap - at), "%s%s", at ? " " : "", name);
			o += reclen;
		}
	}
	l_close((int)fd);
}

/* what Linux made of the hardware: block devices and RTCs, read once */
/* Print a Linux file (debugfs, sysfs) line by line with a prefix. */
static void l_cat(const char *path, const char *prefix)
{
	static char buf[4096];
	long fd = l_open(path, 0), n, at = 0;
	if (fd < 0)
		return;
	while (at < (long)sizeof buf - 1 && (n = l_read((int)fd, buf + at, (long)sizeof buf - 1 - at)) > 0)
		at += n;
	l_close((int)fd);
	buf[at] = 0;
	char *save = NULL;
	for (char *l = strtok_r(buf, "\n", &save); l; l = strtok_r(NULL, "\n", &save))
		printf("%s%s\n", prefix, l);
}

/* A whole disk, not a partition or a boot/RPMB area: vda, sda, mmcblk0. */
static int is_disk(const char *n)
{
	if (!strncmp(n, "mmcblk", 6)) {
		for (const char *c = n + 6; *c; c++)
			if (*c < '0' || *c > '9')
				return 0;
		return n[6] != 0;
	}
	if (!strncmp(n, "loop", 4) || !strncmp(n, "ram", 3) || !strncmp(n, "zram", 4) || !strncmp(n, "nbd", 3))
		return 0;
	size_t l = strlen(n);
	return l && (n[l - 1] < '0' || n[l - 1] > '9');
}

/* One line of a Linux file, without its newline. */
static void l_gets(const char *path, char *out, int cap)
{
	out[0] = 0;
	long fd = l_open(path, 0);
	if (fd < 0)
		return;
	long n = l_read((int)fd, out, cap - 1);
	l_close((int)fd);
	if (n < 0)
		n = 0;
	out[n] = 0;
	char *nl = strchr(out, '\n');
	if (nl)
		*nl = 0;
}

/* ---- Linux's input devices, handed to Flare ----
 * The composition names Flare input devices the host feeds ([linux] inputs,
 * e.g. "touch" and "keys"): Linux's touch screen feeds the one whose name
 * contains "touch", its key devices (power key, volume keys...) together the
 * one whose name contains "key". Each is described as Linux's driver
 * describes it (evdev ioctls inside Linux) and gets its events as they come.
 * flare_abi::proto::input::Describe, laid out as #[repr(C)]. */
struct flare_abs {
	int32_t min, max, fuzz, flat, res;
};
struct flare_describe {
	char name[64];
	uint16_t bustype, vendor, product, version;
	uint32_t ev_bits;
	uint8_t key_bits[96], rel_bits[2], abs_bits[8], prop_bits[4];
	struct flare_abs abs[64];
};
struct flare_event {
	uint16_t kind, code;
	int32_t value;
};
#define FLAREHW_INPUT 0x4604
struct flare_input_req {
	uint32_t index, op;
	uint64_t ptr, len;
};

static int flare_input(int idx, int op, const void *p, size_t len)
{
	struct flare_input_req r = { (uint32_t)idx, (uint32_t)op, (uint64_t)(uintptr_t)p, len };
	return ioctl(hw, FLAREHW_INPUT, &r);
}

#define EVIOC(nr, len) (0x80000000u | ((unsigned)(len) << 16) | ('E' << 8) | (nr))

struct watched {
	char dev[32], name[64];
	int slot; /* Flare input index, or -1: shown only */
	/* A touch screen handed over as an absolute pointer: its axis ranges
	 * (ABS_X, ABS_Y), scaled to 0..32767 (see tablet_describe). */
	int tablet, min[2], max[2];
};

/* A touch screen as Flare's user interfaces take touch (QEMU's virtio
 * tablet): ABS_X and ABS_Y in 0..32767, BTN_LEFT while touched. Linux's
 * driver reports the first finger that way too (pointer emulation), in the
 * panel's own units, with BTN_TOUCH. */
static void tablet_describe(const struct flare_describe *in, struct flare_describe *out)
{
	memset(out, 0, sizeof *out);
	memcpy(out->name, in->name, sizeof out->name);
	out->bustype = in->bustype, out->vendor = in->vendor, out->product = in->product, out->version = in->version;
	out->ev_bits = 1 << 0 | 1 << 1 | 1 << 3;  /* SYN, KEY, ABS */
	out->key_bits[0x110 / 8] |= 1 << (0x110 % 8); /* BTN_LEFT */
	out->abs_bits[0] = 3;                      /* ABS_X, ABS_Y */
	out->abs[0] = (struct flare_abs){ 0, 32767, 0, 0, 0 };
	out->abs[1] = (struct flare_abs){ 0, 32767, 0, 0, 0 };
}

/* One event of a touch screen, as the tablet: 0 to drop it. */
static int tablet_event(const struct watched *w, struct flare_event *e)
{
	if (e->kind == 0)
		return 1;
	if (e->kind == 3 && e->code < 2) {
		int span = w->max[e->code] - w->min[e->code];
		long v = span > 0 ? (long)(e->value - w->min[e->code]) * 32767 / span : 0;
		e->value = (int32_t)(v < 0 ? 0 : v > 32767 ? 32767 : v);
		return 1;
	}
	if (e->kind == 1 && (e->code == 0x14a || e->code == 0x110)) { /* BTN_TOUCH, BTN_LEFT */
		e->code = 0x110;
		return 1;
	}
	return 0;
}

/* Volume up and down held together: the kernel console takes the screen
 * over the user interface (the boot log, timings, the profile), or gives
 * it back. */
static void volume_keys(unsigned code, int value)
{
	static int held[2];
	static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
	pthread_mutex_lock(&lock);
	int was = held[0] && held[1];
	held[code == 115] = value != 0;
	int now = held[0] && held[1];
	pthread_mutex_unlock(&lock);
	if (now && !was) {
		int fd = open("/dev/flarectl", O_WRONLY);
		if (fd >= 0) {
			if (write(fd, "console toggle", 14) < 0)
				printf("linuxdrv: console toggle: %s\n", strerror(errno));
			close(fd);
		}
	}
}

static void *input_thread(void *p)
{
	struct watched *w = p;
	char path[64];
	snprintf(path, sizeof path, "/dev/input/%s", w->dev);
	long fd = l_open(path, 0);
	if (fd < 0)
		return NULL;
	int x = -1, y = -1, down = 0, touches = 0;
	for (;;) {
		struct {
			long sec, usec;
			unsigned short type, code;
			int value;
		} ev[64];
		long n = l_read((int)fd, ev, sizeof ev);
		if (n <= 0)
			break;
		struct flare_event out[64];
		int m = 0;
		for (long i = 0; i < n / (long)sizeof ev[0]; i++) {
			out[m] = (struct flare_event){ ev[i].type, ev[i].code, ev[i].value };
			if (!w->tablet || tablet_event(w, &out[m]))
				m++;
			if (ev[i].type == 3 && (ev[i].code == 0x35 || ev[i].code == 0x00))
				x = ev[i].value; /* ABS_MT_POSITION_X, ABS_X */
			else if (ev[i].type == 3 && (ev[i].code == 0x36 || ev[i].code == 0x01))
				y = ev[i].value;
			else if (ev[i].type == 1 && ev[i].code == 0x14a) /* BTN_TOUCH */
				down = ev[i].value ? 1 : 0;
			else if (ev[i].type == 1 && ev[i].value == 1)
				printf("linuxdrv: %s: key %u pressed\n", w->name, ev[i].code);
			if (ev[i].type == 1 && (ev[i].code == 114 || ev[i].code == 115))
				volume_keys(ev[i].code, ev[i].value);
			else if (ev[i].type == 0 && down == 1) {
				if (++touches <= 3) /* the first few: proof it works */
					printf("linuxdrv: %s: touch %d at %d,%d\n", w->name, touches, x, y);
				down = 2;
			}
		}
		if (w->slot >= 0 && m)
			flare_input(w->slot, 1, out, (size_t)m * sizeof out[0]);
		fflush(stdout);
	}
	l_close((int)fd);
	return NULL;
}

/* What a Linux input device is, from Linux's own evdev. */
static void describe_linux(const char *dev, struct flare_describe *d)
{
	char path[64];
	snprintf(path, sizeof path, "/dev/input/%s", dev);
	memset(d, 0, sizeof *d);
	long fd = l_open(path, 0);
	if (fd < 0)
		return;
	l_sys(NR_IOCTL, fd, EVIOC(0x06, sizeof d->name - 1), (long)d->name, 0, 0);
	uint16_t id[4] = { 0 };
	l_sys(NR_IOCTL, fd, EVIOC(0x02, sizeof id), (long)id, 0, 0);
	d->bustype = id[0], d->vendor = id[1], d->product = id[2], d->version = id[3];
	l_sys(NR_IOCTL, fd, EVIOC(0x20, 4), (long)&d->ev_bits, 0, 0);
	l_sys(NR_IOCTL, fd, EVIOC(0x21, sizeof d->key_bits), (long)d->key_bits, 0, 0);
	l_sys(NR_IOCTL, fd, EVIOC(0x22, sizeof d->rel_bits), (long)d->rel_bits, 0, 0);
	l_sys(NR_IOCTL, fd, EVIOC(0x23, sizeof d->abs_bits), (long)d->abs_bits, 0, 0);
	l_sys(NR_IOCTL, fd, EVIOC(0x09, sizeof d->prop_bits), (long)d->prop_bits, 0, 0);
	for (int a = 0; a < 64; a++) {
		if (!(d->abs_bits[a / 8] >> (a % 8) & 1))
			continue;
		int32_t ai[6] = { 0 }; /* value, min, max, fuzz, flat, resolution */
		if (l_sys(NR_IOCTL, fd, EVIOC(0x40 + a, sizeof ai), (long)ai, 0, 0) == 0)
			d->abs[a] = (struct flare_abs){ ai[1], ai[2], ai[3], ai[4], ai[5] };
	}
	l_close((int)fd);
}

/* A touch screen (multi-touch, or a direct device) or another absolute
 * pointer (QEMU's virtio tablet: ABS_X and ABS_Y). */
static int is_touch(const struct flare_describe *d)
{
	return (d->abs_bits[0x35 / 8] >> (0x35 % 8) & 1) || (d->prop_bits[0] & 2 /* INPUT_PROP_DIRECT */) ||
	       (d->abs_bits[0] & 3) == 3;
}

/* The Flare input whose name contains `what`, or -1. */
static int flare_slot(const char *what)
{
	for (int i = 0; i < 16; i++) {
		char name[32] = "";
		if (flare_input(i, 2, name, sizeof name))
			return -1;
		if (strstr(name, what))
			return i;
	}
	return -1;
}

/* Hand Linux's input devices to Flare and watch them. `devs`: event names. */
static void bridge_inputs(char *devs[], int n)
{
	int touch = flare_slot("touch"), keys = flare_slot("key");
	struct flare_describe *kd = calloc(1, sizeof *kd);
	int nkeys = 0;
	for (int i = 0; i < n; i++) {
		struct flare_describe d;
		describe_linux(devs[i], &d);
		int slot = -1;
		int tablet = 0;
		if (is_touch(&d) && touch >= 0) {
			struct flare_describe t;
			tablet_describe(&d, &t);
			flare_input(touch, 0, &t, sizeof t);
			printf("linuxdrv: %s (%s) is Flare's touch screen (%d..%d x %d..%d)\n", devs[i], d.name,
			       d.abs[0].min, d.abs[0].max, d.abs[1].min, d.abs[1].max);
			slot = touch;
			touch = -1; /* one touch screen */
			tablet = 1;
		} else if ((d.ev_bits & 2) && keys >= 0 && kd) {
			/* keys: one Flare device for all of Linux's key devices */
			kd->ev_bits |= d.ev_bits;
			for (int b = 0; b < 96; b++)
				kd->key_bits[b] |= d.key_bits[b];
			nkeys++;
			slot = keys;
		}
		struct watched *w = calloc(1, sizeof *w);
		if (!w)
			continue;
		snprintf(w->dev, sizeof w->dev, "%s", devs[i]);
		snprintf(w->name, sizeof w->name, "%s", d.name[0] ? d.name : devs[i]);
		w->slot = slot;
		w->tablet = tablet;
		w->min[0] = d.abs[0].min, w->max[0] = d.abs[0].max;
		w->min[1] = d.abs[1].min, w->max[1] = d.abs[1].max;
		pthread_t t;
		if (pthread_create(&t, NULL, input_thread, w) == 0)
			pthread_detach(t);
	}
	if (nkeys && kd) {
		snprintf(kd->name, sizeof kd->name, "linuxdrv keys");
		kd->ev_bits |= 1; /* EV_SYN */
		flare_input(keys, 0, kd, sizeof *kd);
		printf("linuxdrv: %d key device(s) are Flare's keys\n", nkeys);
	}
	free(kd);
	/* /etc/rc waits for this before starting a user interface. */
	printf("linuxdrv: [%.2f s] inputs ready\n", since_boot());
	devices_ready = 1;
	slowest_initcalls();
	FILE *f = fopen("/tmp/linuxdrv.ready", "w");
	if (f)
		fclose(f);
}

/* ---- Flare's root filesystem from storage ----
 * A phone's boot image carries only a minimal root filesystem; the full one
 * (kernel/src/ramdisk.rs format, zlib-compressed) is on the userdata
 * partition (`fastboot flash userdata userdata.img`). Read it through
 * Linux's eMMC driver into DMA memory and hand it to the kernel, which
 * inflates it and adds its files. */
#define FLAREHW_ROOTFS 0x4605

/* Whether `dev`'s partition name (GPT) is `want`. */
static int partname_is(const char *dev, const char *want)
{
	static char buf[512];
	char path[96];
	snprintf(path, sizeof path, "/sys/class/block/%s/uevent", dev);
	long fd = l_open(path, 0);
	if (fd < 0)
		return 0;
	long n = l_read((int)fd, buf, sizeof buf - 1);
	l_close((int)fd);
	if (n <= 0)
		return 0;
	buf[n] = 0;
	char key[64];
	snprintf(key, sizeof key, "PARTNAME=%s\n", want);
	return strstr(buf, key) != NULL;
}

static int has_rootfs_header(const char *dev, uint64_t *total)
{
	char path[64];
	unsigned char h[32];
	snprintf(path, sizeof path, "/dev/%s", dev);
	long fd = l_open(path, 0);
	if (fd < 0)
		return 0;
	long r = l_read((int)fd, h, sizeof h);
	l_close((int)fd);
	if (r != 32 || memcmp(h, "FLARERD", 7) || (h[7] != '1' && h[7] != 'Z' && h[7] != 'C'))
		return 0;
	uint64_t len;
	memcpy(&len, h + 8, 8);
	*total = 32 + len;
	return 1;
}

/* The userdata partition, else (QEMU) a disk that is the image itself; ""
 * if neither. `*part`: a partition named userdata exists (with or without
 * the image). */
static void find_rootfs(char *found, size_t cap, uint64_t *total, int *part)
{
	static char all[4096];
	l_list("/sys/class/block", all, sizeof all);
	found[0] = 0;
	*part = 0;
	char *save = NULL, *list = strdup(all);
	for (char *n = strtok_r(list, " ", &save); n && !found[0]; n = strtok_r(NULL, " ", &save))
		if (partname_is(n, "userdata")) {
			*part = 1;
			if (has_rootfs_header(n, total))
				snprintf(found, cap, "%s", n);
		}
	free(list);
	list = strdup(all);
	save = NULL;
	for (char *n = strtok_r(list, " ", &save); n && !found[0]; n = strtok_r(NULL, " ", &save))
		if (is_disk(n) && has_rootfs_header(n, total))
			snprintf(found, cap, "%s", n);
	free(list);
}

#define FLAREHW_DMA_FREE 0x4607

/* FLARERDC: zlib pieces (xtask's pack_ramdisk), inflated by one thread per
 * CPU into a fresh DMA block as a plain FLARERD1 archive */
struct piece { uint64_t off, len, out, olen; };
static struct {
	const unsigned char *body;
	unsigned char *out;
	const struct piece *p;
	uint32_t n;
	int next, bad;
	uint64_t sum;
	pthread_mutex_t lock;
} inf = { .lock = PTHREAD_MUTEX_INITIALIZER };

static void *inflate_worker(void *arg)
{
	(void)arg;
	for (;;) {
		pthread_mutex_lock(&inf.lock);
		int i = inf.next++;
		pthread_mutex_unlock(&inf.lock);
		if (i >= (int)inf.n)
			return NULL;
		const struct piece *p = &inf.p[i];
		uLongf olen = p->olen;
		int ok = uncompress(inf.out + 32 + p->out, &olen, inf.body + p->off, p->len) == Z_OK && olen == p->olen;
		uint64_t sum = 0;
		pthread_mutex_lock(&inf.lock);
		inf.bad |= !ok;
		inf.sum += sum;
		pthread_mutex_unlock(&inf.lock);
	}
}

static unsigned char *inflate_rootfs(const unsigned char *img, uint64_t total, uint64_t *phys, uint64_t *size)
{
	struct timespec t0, t1;
	clock_gettime(CLOCK_MONOTONIC, &t0);
	uint64_t ulen;
	memcpy(&ulen, img + 24, 8);
	const unsigned char *body = img + 32;
	uint32_t n;
	memcpy(&n, body, 4);
	if (32 + 8 + (uint64_t)n * 32 > total) {
		printf("linuxdrv: the root filesystem's piece table is damaged\n");
		return NULL;
	}
	*size = (32 + ulen + 0xfff) & ~0xfffull;
	if (dma_alloc(*size, phys)) {
		printf("linuxdrv: no %llu MiB of DMA memory to inflate the root filesystem\n", (unsigned long long)(*size >> 20));
		return NULL;
	}
	unsigned char *out = mmap(NULL, *size, PROT_READ | PROT_WRITE, MAP_SHARED, hw, (off_t)*phys);
	if (out == MAP_FAILED)
		return NULL;
	inf.body = body;
	inf.out = out;
	inf.p = (const struct piece *)(body + 8);
	inf.n = n;
	inf.next = 0;
	inf.bad = 0;
	inf.sum = 0;
	pthread_t t[8];
	int nt = 8;
	for (int i = 0; i < nt; i++)
		pthread_create(&t[i], NULL, inflate_worker, NULL);
	for (int i = 0; i < nt; i++)
		pthread_join(t[i], NULL);
	if (inf.bad) {
		printf("linuxdrv: the root filesystem does not inflate (damaged or stale userdata.img)\n");
		munmap(out, *size);
		return NULL;
	}
	memcpy(out, "FLARERD1", 8);
	memcpy(out + 8, &ulen, 8);
	/* 0: verified here (each piece's zlib stream has its checksum) */
	memset(out + 16, 0, 8);
	memset(out + 24, 0, 8);
	clock_gettime(CLOCK_MONOTONIC, &t1);
	printf("linuxdrv: [%.2f s] %llu MiB root filesystem inflated on %d threads in %ld ms\n", since_boot(),
	       (unsigned long long)(ulen >> 20), nt,
	       (long)((t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000));
	return out;
}

static void load_rootfs(void)
{
	/* A disk shows before Linux has read its partition table: look again
	 * until the userdata partition is there (or 20 s). */
	char found[32] = "";
	uint64_t total = 0;
	int part = 0;
	for (int i = 0; i < 40; i++) {
		find_rootfs(found, sizeof found, &total, &part);
		if (found[0] || part)
			break;
		struct timespec ts = { 0, 500000000 };
		nanosleep(&ts, NULL);
	}
	if (!found[0]) {
		printf(part ? "linuxdrv: the userdata partition has no Flare root filesystem (flash userdata.img)\n"
			    : "linuxdrv: no userdata partition (and no disk with a Flare root filesystem)\n");
		return;
	}
	struct timespec t0, t1;
	clock_gettime(CLOCK_MONOTONIC, &t0);
	uint64_t size = (total + 0xfff) & ~0xfffull, phys;
	if (dma_alloc(size, &phys)) {
		printf("linuxdrv: no %llu MiB of DMA memory for the root filesystem\n", (unsigned long long)(size >> 20));
		return;
	}
	unsigned char *buf = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, hw, (off_t)phys);
	if (buf == MAP_FAILED) {
		printf("linuxdrv: cannot map the root filesystem buffer: %s\n", strerror(errno));
		return;
	}
	char path[64];
	snprintf(path, sizeof path, "/dev/%s", found);
	long fd = l_open(path, 0);
	/* big requests: 8 MiB readahead instead of 128 KiB (fewer, longer
	 * transfers from the eMMC) */
	if (fd >= 0)
		l_sys(NR_IOCTL, fd, 0x1262 /* BLKRASET */, 16384 /* sectors */, 0, 0);
	uint64_t got = 0;
	while (fd >= 0 && got < total) {
		long chunk = total - got > (8 << 20) ? (8 << 20) : (long)(total - got);
		long r = l_read((int)fd, buf + got, chunk);
		if (r <= 0)
			break;
		got += (uint64_t)r;
	}
	if (fd >= 0)
		l_close((int)fd);
	clock_gettime(CLOCK_MONOTONIC, &t1);
	long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
	if (got < total) {
		printf("linuxdrv: %s: read %llu of %llu bytes of the root filesystem\n", found, (unsigned long long)got,
		       (unsigned long long)total);
		return;
	}
	printf("linuxdrv: [%.2f s] %s: %llu MiB root filesystem read through Linux's driver in %ld ms\n", since_boot(),
	       found, (unsigned long long)(total >> 20), ms);
	if (!memcmp(buf, "FLARERDC", 8)) {
		/* in pieces: inflated here on every CPU, into a plain archive the
		 * kernel takes as it is (no work under its lock) */
		uint64_t nphys, nsize;
		unsigned char *nbuf = inflate_rootfs(buf, total, &nphys, &nsize);
		munmap(buf, size);
		uint64_t fr[2] = { phys, size };
		if (ioctl(hw, FLAREHW_DMA_FREE, fr))
			printf("linuxdrv: could not give back the compressed root filesystem: %s\n", strerror(errno));
		if (!nbuf)
			return;
		buf = nbuf;
		phys = nphys;
		size = nsize;
		memcpy(&total, buf + 8, 8);
		total += 32;
	}
	uint64_t req[2] = { phys, total };
	int n = ioctl(hw, FLAREHW_ROOTFS, req);
	if (n < 0)
		printf("linuxdrv: Flare refused the root filesystem: %s\n", strerror(errno));
	else
		printf("linuxdrv: root filesystem added to Flare: %d entries\n", n);
	munmap(buf, size);
}

/* ---- a serial device for Flare over a Linux tty ----
 * [linux] serials = ["fdb"]: Flare's /dev/fdb carried over a tty of this
 * Linux: the USB serial gadget on a phone (the computer's /dev/ttyACM0),
 * QEMU's virtio console in tests. Raw mode; bytes both ways. */
#define FLAREHW_SERIAL 0x4606

static long flare_serial(int op, void *p, size_t len)
{
	struct { uint32_t index, op; uint64_t ptr, len; } r = { 0, (uint32_t)op, (uint64_t)(uintptr_t)p, len };
	return ioctl(hw, FLAREHW_SERIAL, &r);
}

static long tty_fd = -1;

static void *serial_from_tty(void *p)
{
	(void)p;
	for (;;) {
		char b[4096];
		long n = l_read((int)tty_fd, b, sizeof b);
		if (n <= 0) {
			usleep(10000);
			continue;
		}
		for (long done = 0; done < n;) {
			long m = flare_serial(0, b + done, (size_t)(n - done));
			if (m <= 0) {
				usleep(1000); /* Flare's buffer is full: let its reader catch up */
				continue;
			}
			done += m;
		}
	}
	return NULL;
}

static void *serial_to_tty(void *p)
{
	(void)p;
	for (;;) {
		char b[4096];
		long n = flare_serial(1, b, sizeof b);
		if (n <= 0) {
			usleep(2000);
			continue;
		}
		for (long done = 0; done < n;) {
			long m = l_sys(NR_WRITE, tty_fd, (long)(b + done), n - done, 0, 0);
			if (m <= 0) {
				usleep(1000);
				continue;
			}
			done += m;
		}
	}
	return NULL;
}

static void bridge_serial(void)
{
	char probe[1];
	if (flare_serial(1, probe, 0) < 0)
		return; /* no [linux] serials */
	static const char *const ttys[] = { "ttyGS0", "hvc0" };
	const char *name = NULL;
	/* the USB gadget's tty appears once the controller is up */
	for (int i = 0; i < 20 && !name; i++) {
		for (unsigned k = 0; k < sizeof ttys / sizeof ttys[0] && !name; k++) {
			char path[32];
			snprintf(path, sizeof path, "/dev/%s", ttys[k]);
			long fd = l_open(path, 2 /* O_RDWR */ | 0400 /* O_NOCTTY */);
			if (fd >= 0) {
				tty_fd = fd;
				name = ttys[k];
			}
		}
		if (!name)
			usleep(500000);
	}
	if (!name) {
		printf("linuxdrv: no tty for Flare's serial device (USB gadget not up)\n");
		return;
	}
	/* raw: struct termios (asm-generic): iflag, oflag, cflag, lflag, line, cc[19] */
	struct { uint32_t iflag, oflag, cflag, lflag; uint8_t line, cc[19]; } t;
	if (l_sys(NR_IOCTL, tty_fd, 0x5401 /* TCGETS */, (long)&t, 0, 0) == 0) {
		t.iflag = 0;
		t.oflag = 0;
		t.lflag = 0;
		t.cflag = (t.cflag & ~0x30u) | 0x30 /* CS8 */ | 0x80 /* CREAD */ | 0x800 /* CLOCAL */;
		t.cc[6] = 1; /* VMIN */
		t.cc[5] = 0; /* VTIME */
		l_sys(NR_IOCTL, tty_fd, 0x5402 /* TCSETS */, (long)&t, 0, 0);
	}
	pthread_t a, b;
	pthread_create(&a, NULL, serial_from_tty, NULL);
	pthread_create(&b, NULL, serial_to_tty, NULL);
	pthread_detach(a);
	pthread_detach(b);
	printf("linuxdrv: Flare's serial device carried over Linux's /dev/%s\n", name);
}

static void report(void)
{
	static char names[4096];
	/* The root filesystem first, as soon as its partition is there (the
	 * user interface waits for it); the other devices keep probing. */
	load_rootfs();
	/* A phone's devices appear as their suppliers do (the RPM, then the
	 * clocks, then eMMC, I2C...), each probe deferred until the last: wait
	 * until nothing is waiting any more (the touch screen among them), or
	 * 10 s. */
	static char deferred[256];
	long ndeferred = 0;
	for (int i = 0; i < 50; i++) {
		long fd = l_open("/sys/kernel/debug/devices_deferred", 0);
		ndeferred = 0;
		if (fd >= 0) {
			ndeferred = l_read((int)fd, deferred, sizeof deferred - 1);
			l_close((int)fd);
		}
		l_list("/sys/class/block", names, sizeof names);
		if (ndeferred <= 0 && names[0])
			break;
		/* devices that wait for what the driver host starts later (the
		 * GPU's IOMMU, the DSP...) keep waiting: settled once the list
		 * stays the same for a second */
		static char last[256];
		static int same;
		deferred[ndeferred > 0 ? ndeferred : 0] = 0;
		same = strcmp(last, deferred) == 0 ? same + 1 : 0;
		snprintf(last, sizeof last, "%s", deferred);
		if (same >= 5 && names[0])
			break;
		struct timespec ts = { 0, 200000000 };
		nanosleep(&ts, NULL);
	}
	/* (what kept the user interface waiting the whole 10 s) */
	if (ndeferred > 0) {
		deferred[ndeferred] = 0;
		for (char *c = deferred; *c; c++)
			if (*c == '\n' || *c == '\t')
				*c = ' ';
		printf("linuxdrv: still waiting: %s\n", deferred);
	}
	/* (again: the partitions are there now) */
	l_list("/sys/class/block", names, sizeof names);
	/* The whole disks (a phone's eMMC has dozens of partitions). */
	static char disks[512];
	int nparts = 0, at = 0;
	disks[0] = 0;
	char *save = NULL;
	for (char *n = strtok_r(names, " ", &save); n; n = strtok_r(NULL, " ", &save)) {
		if (is_disk(n)) {
			if (at + (int)strlen(n) + 2 < (int)sizeof disks)
				at += snprintf(disks + at, sizeof disks - (size_t)at, "%s%s", at ? " " : "", n);
		} else if (strncmp(n, "loop", 4) && strncmp(n, "ram", 3) && strncmp(n, "zram", 4) && strncmp(n, "nbd", 3)) {
			nparts++;
		}
	}
	printf("linuxdrv: [%.2f s] block devices: %s", since_boot(), disks[0] ? disks : "none");
	if (nparts)
		printf(" (and %d partitions)", nparts);
	printf("\n");
	save = NULL;
	for (char *n = strtok_r(disks, " ", &save); n; n = strtok_r(NULL, " ", &save)) {
		char path[64];
		unsigned char sec[512];
		snprintf(path, sizeof path, "/dev/%s", n);
		long fd = l_open(path, 0);
		if (fd < 0)
			continue;
		long r = l_read((int)fd, sec, sizeof sec);
		l_close((int)fd);
		if (r == 512)
			printf("linuxdrv: %s: first sector read through Linux's driver: %02x %02x %02x %02x ... boot signature %02x%02x\n",
			       n, sec[0], sec[1], sec[2], sec[3], sec[510], sec[511]);
		/* Then past the cache (flushed before each read): each read is
		 * a request to the device and an interrupt back, which the first,
		 * cached read may not have needed. Once, on the first disk. */
		static int direct_done;
		if (r == 512 && !direct_done) {
			direct_done = 1;
			static unsigned char buf[512];
			fd = l_open(path, 0);
			int ok = 0;
			for (int k = 0; fd >= 0 && k < 8; k++) {
				l_sys(NR_IOCTL, fd, L_BLKFLSBUF, 0, 0, 0);
				if (l_sys(NR_PREAD64, fd, (long)buf, 512, (long)k * 4096, 0) == 512)
					ok++;
			}
			if (fd >= 0)
				l_close((int)fd);
			printf("linuxdrv: %s: %d of 8 uncached reads (one device interrupt each)\n", n, ok);
		}
	}
	l_list("/sys/class/rtc", names, sizeof names);
	if (names[0]) {
		long fd = l_open("/dev/rtc0", 0);
		int tm[9] = { 0 };
		if (fd >= 0 && l_sys(NR_IOCTL, fd, 0x80247009 /* RTC_RD_TIME */, (long)tm, 0, 0) == 0)
			printf("linuxdrv: rtc0 (Linux's driver): %04d-%02d-%02d %02d:%02d:%02d UTC\n", tm[5] + 1900, tm[4] + 1,
			       tm[3], tm[2], tm[1], tm[0]);
		if (fd >= 0)
			l_close((int)fd);
	}
	l_list("/sys/class/input", names, sizeof names);
	char *devs[32];
	int ndev = 0;
	save = NULL;
	for (char *n = strtok_r(names, " ", &save); n && ndev < 32; n = strtok_r(NULL, " ", &save)) {
		if (strncmp(n, "event", 5))
			continue;
		char path[96], dname[64] = "";
		snprintf(path, sizeof path, "/sys/class/input/%s/device/name", n);
		l_gets(path, dname, sizeof dname);
		printf("%s%s (%s)", ndev ? ", " : "linuxdrv: input devices: ", n, dname[0] ? dname : "?");
		devs[ndev++] = n;
	}
	printf(ndev ? "\n" : "linuxdrv: input devices: none\n");
	bridge_serial();
	bridge_inputs(devs, ndev);
	/* Bring-up: what each device Linux has not probed yet still waits for. */
	l_cat("/sys/kernel/debug/devices_deferred", "linuxdrv: waiting: ");
	fflush(stdout);
}

/* ---- The rest of the phone: what goes through the secure world ----
 * The board's linux.txt hides the devices whose firmware the secure world
 * loads (the GPU, its IOMMU, Wi-Fi, the audio DSP): their "qcom,"
 * compatibles start with a capital ("Qcom,adreno"), so no driver takes them (as
 * QRT does on this phone). Once the phone shell is up, the driver host
 * mounts the phone's partitions, lays out their firmware for Linux, and
 * gives each stage of /etc/linuxdrv/stages back in turn: the compatibles
 * restored in the tree Linux reads (it points into this buffer), then every
 * device probed. Remote processors (Wi-Fi, the DSP) are started. */
static char *dtb;
static size_t dtb_len;
#define NR_UMOUNT2 39
#define NR_SYMLINKAT 36

static long l_write_text(const char *path, const char *text)
{
	long fd = l_open(path, 1 /* O_WRONLY */);
	if (fd < 0)
		return fd;
	long r = l_sys(NR_WRITE, fd, (long)text, (long)strlen(text), 0, 0);
	l_close((int)fd);
	return r;
}

static int l_exists(const char *path)
{
	long fd = l_open(path, 0x200000 /* O_PATH */);
	if (fd < 0)
		return 0;
	l_close((int)fd);
	return 1;
}

static void l_mkdirs(const char *path)
{
	char p[160];
	snprintf(p, sizeof p, "%s", path);
	for (char *c = p + 1; *c; c++)
		if (*c == '/') {
			*c = 0;
			l_sys(NR_MKDIRAT, AT_FDCWD, (long)p, 0755, 0, 0);
			*c = '/';
		}
	l_sys(NR_MKDIRAT, AT_FDCWD, (long)p, 0755, 0, 0);
}

/* copy a file inside Linux; its size, or a negative error */
static long l_copy(const char *from, const char *to)
{
	long in = l_open(from, 0);
	if (in < 0)
		return in;
	char dir[160];
	snprintf(dir, sizeof dir, "%s", to);
	char *slash = strrchr(dir, '/');
	if (slash) {
		*slash = 0;
		l_mkdirs(dir);
	}
	long out = l_sys(NR_OPENAT, AT_FDCWD, (long)to, 0x241 /* O_WRONLY|O_CREAT|O_TRUNC */, 0644, 0);
	if (out < 0) {
		l_close((int)in);
		return out;
	}
	static char buf[65536];
	long total = 0, n;
	while ((n = l_read((int)in, buf, sizeof buf)) > 0) {
		l_sys(NR_WRITE, out, (long)buf, n, 0, 0);
		total += n;
	}
	l_close((int)in);
	l_close((int)out);
	return total;
}

/* The phone's partitions Linux's drivers need files from, read-only:
 * /mnt/<partition name>. */
static void mount_partitions(void)
{
	static const char *const want[] = { "modem_a", "modem", "vendor_a", "vendor", "persist", NULL };
	static char names[8192];
	l_list("/sys/class/block", names, sizeof names);
	l_mkdirs("/mnt");
	char *save = NULL;
	for (char *dev = strtok_r(names, " ", &save); dev; dev = strtok_r(NULL, " ", &save)) {
		char path[96], ue[512] = "", part[40] = "";
		snprintf(path, sizeof path, "/sys/class/block/%s/uevent", dev);
		long fd = l_open(path, 0);
		if (fd < 0)
			continue;
		long n = l_read((int)fd, ue, sizeof ue - 1);
		l_close((int)fd);
		ue[n > 0 ? n : 0] = 0;
		const char *pn = strstr(ue, "PARTNAME=");
		if (!pn)
			continue;
		int i = 0;
		for (pn += 9; *pn && *pn != '\n' && i < 39; pn++)
			part[i++] = *pn;
		part[i] = 0;
		int wanted = 0;
		for (int w = 0; want[w]; w++)
			wanted |= !strcmp(part, want[w]);
		if (!wanted)
			continue;
		char mnt[64], devp[64];
		snprintf(mnt, sizeof mnt, "/mnt/%s", part);
		snprintf(devp, sizeof devp, "/dev/%s", dev);
		l_mkdirs(mnt);
		static const char *const fss[] = { "ext4", "vfat", NULL };
		const char *got = NULL;
		for (int f = 0; fss[f] && !got; f++)
			if (l_sys(NR_MOUNT, (long)devp, (long)mnt, (long)fss[f], 1 /* MS_RDONLY */, 0) == 0)
				got = fss[f];
		printf("linuxdrv: %s (%s): %s\n", part, dev, got ? got : "not mountable");
	}
}

/* the first of `parts`/`files` that exists, copied to `to` */
static long copy_first(const char *const parts[], const char *const files[], const char *to)
{
	for (int p = 0; parts[p]; p++)
		for (int f = 0; files[f]; f++) {
			char from[192];
			snprintf(from, sizeof from, "/mnt/%s/%s", parts[p], files[f]);
			long n = l_copy(from, to);
			if (n > 0) {
				printf("linuxdrv: firmware %s (from %s)\n", to + 14, from);
				return n;
			}
		}
	return -1;
}

/* Lay out the firmware Linux's drivers ask for. */
static void firmware(void)
{
	static const char *const vendor[] = { "vendor_a", "vendor", NULL };
	static const char *const modem[] = { "modem_a", "modem", NULL };
	/* the DSP's and Wi-Fi's firmware (adsp.mdt, wcnss.mdt and their
	 * pieces): where the phone keeps them, as Linux's search path */
	for (int m = 0; modem[m]; m++) {
		char img[64];
		snprintf(img, sizeof img, "/mnt/%s/image", modem[m]);
		if (l_exists(img)) {
			l_write_text("/sys/module/firmware_class/parameters/path", img);
			printf("linuxdrv: firmware from %s\n", img);
			break;
		}
	}
	/* the GPU's command processor microcode and its zap shader */
	static const char *const gpu[] = { "a530_pm4.fw", "a530_pfp.fw", "a506_zap.mdt", "a506_zap.b00", "a506_zap.b01",
					   "a506_zap.b02", NULL };
	for (int g = 0; gpu[g]; g++) {
		char f1[64], f2[64], f3[64], to[128];
		snprintf(f1, sizeof f1, "firmware/%s", gpu[g]);
		snprintf(f2, sizeof f2, "etc/firmware/%s", gpu[g]);
		snprintf(f3, sizeof f3, "image/%s", gpu[g]);
		const char *const files[] = { f1, f2, NULL };
		const char *const files_m[] = { f3, NULL };
		snprintf(to, sizeof to, "/lib/firmware/qcom/%s", gpu[g]);
		if (copy_first(vendor, files, to) < 0)
			copy_first(modem, files_m, to);
		if (!strncmp(gpu[g], "a506_zap", 8)) {
			char to2[128];
			snprintf(to2, sizeof to2, "/lib/firmware/qcom/msm8953/xiaomi/tissot/%s", gpu[g]);
			l_copy(to, to2);
		}
	}
	/* Wi-Fi's calibration (NV) */
	static const char *const nvp[] = { "vendor_a", "vendor", "persist", NULL };
	static const char *const nvf[] = { "firmware/wlan/prima/WCNSS_qcom_wlan_nv.bin", "etc/wifi/WCNSS_qcom_wlan_nv.bin",
					   "WCNSS_qcom_wlan_nv.bin", NULL };
	copy_first(nvp, nvf, "/lib/firmware/wlan/prima/WCNSS_qcom_wlan_nv.bin");
}

/* "Qcom,x" back to "qcom,x" in the tree Linux reads (dtb.py trim --hide) */
static int unhide(const char *compat)
{
	size_t n = strlen(compat);
	int hits = 0;
	if (n < 2 || compat[0] < 'a' || compat[0] > 'z')
		return 0;
	for (size_t i = 1; dtb && i + n < dtb_len; i++)
		if (dtb[i] == compat[0] - 32 && !memcmp(dtb + i + 1, compat + 1, n - 1) && dtb[i + n] == 0) {
			dtb[i] = compat[0];
			hits++;
		}
	__sync_synchronize();
	return hits;
}

/* every platform device without a driver: probed again */
static int probe_unbound(char *bound_now, int cap)
{
	static char names[16384];
	l_list("/sys/bus/platform/devices", names, sizeof names);
	int newly = 0, at = 0;
	char *save = NULL;
	bound_now[0] = 0;
	for (char *dev = strtok_r(names, " ", &save); dev; dev = strtok_r(NULL, " ", &save)) {
		char p[160];
		snprintf(p, sizeof p, "/sys/bus/platform/devices/%s/driver", dev);
		if (l_exists(p))
			continue;
		l_write_text("/sys/bus/platform/drivers_probe", dev);
		if (l_exists(p)) {
			newly++;
			if (at + (int)strlen(dev) + 2 < cap)
				at += snprintf(bound_now + at, (size_t)(cap - at), "%s%s", at ? " " : "", dev);
		}
	}
	return newly;
}

/* remote processors that wait: started (the secure world loads them) */
static void start_remoteprocs(void)
{
	static char names[1024];
	l_list("/sys/class/remoteproc", names, sizeof names);
	char *save = NULL;
	for (char *rp = strtok_r(names, " ", &save); rp; rp = strtok_r(NULL, " ", &save)) {
		char sp[96], st[32] = "", fw[64] = "";
		snprintf(sp, sizeof sp, "/sys/class/remoteproc/%s/firmware", rp);
		l_gets(sp, fw, sizeof fw);
		snprintf(sp, sizeof sp, "/sys/class/remoteproc/%s/state", rp);
		l_gets(sp, st, sizeof st);
		if (!strncmp(st, "offline", 7)) {
			printf("linuxdrv: starting %s (%s) through the secure world\n", rp, fw);
			fflush(stdout);
			l_write_text(sp, "start");
			for (int t = 0; t < 100; t++) {
				l_gets(sp, st, sizeof st);
				if (!strncmp(st, "running", 7))
					break;
				usleep(100000);
			}
		}
		printf("linuxdrv: %s (%s): %s\n", rp, fw, st);
	}
}

/* The GPU, asked what it is and whether it runs (msm's GET_PARAM). */
static void gpu_check(void)
{
	static char names[512];
	l_list("/dev/dri", names, sizeof names);
	if (!names[0])
		return;
	char *save = NULL;
	for (char *n = strtok_r(names, " ", &save); n; n = strtok_r(NULL, " ", &save)) {
		char p[64];
		snprintf(p, sizeof p, "/dev/dri/%s", n);
		long fd = l_open(p, 2);
		if (fd < 0)
			continue;
		struct { uint32_t pipe, param; uint64_t value; uint32_t len, pad; } q = { 1 /* 3D */, 1 /* GPU_ID */, 0, 0, 0 };
		long r = l_sys(NR_IOCTL, fd, 0xc0186440 /* DRM_IOCTL_MSM_GET_PARAM */, (long)&q, 0, 0);
		uint64_t id = q.value, t0 = 0, t1 = 0;
		q.param = 4; /* TIMESTAMP */
		if (l_sys(NR_IOCTL, fd, 0xc0186440, (long)&q, 0, 0) == 0)
			t0 = q.value;
		usleep(100000);
		if (l_sys(NR_IOCTL, fd, 0xc0186440, (long)&q, 0, 0) == 0)
			t1 = q.value;
		l_close((int)fd);
		if (r == 0 && id)
			printf("linuxdrv: GPU: Adreno %llu (%s), %s\n", (unsigned long long)id, p,
			       t1 > t0 ? "running" : "present, its clock not counting");
		else
			printf("linuxdrv: %s: not a GPU this asked (%ld)\n", p, r);
		return;
	}
}

/* What Linux drives now, by class. */
static void summary(void)
{
	static char names[4096];
	char line[1024];
	int at;
	l_list("/sys/class/backlight", names, sizeof names);
	if (names[0]) {
		char p[128], v[32] = "", m[32] = "";
		snprintf(p, sizeof p, "/sys/class/backlight/%s/brightness", strtok(names, " "));
		l_gets(p, v, sizeof v);
		snprintf(p + strlen(p) - 10, 20, "max_brightness");
		l_gets(p, m, sizeof m);
		printf("linuxdrv: backlight: %s of %s\n", v, m);
	}
	l_list("/sys/class/power_supply", names, sizeof names);
	at = 0;
	char *save = NULL;
	for (char *n = strtok_r(names, " ", &save); n; n = strtok_r(NULL, " ", &save)) {
		char p[128], cap[16] = "", st[32] = "";
		snprintf(p, sizeof p, "/sys/class/power_supply/%s/capacity", n);
		l_gets(p, cap, sizeof cap);
		snprintf(p, sizeof p, "/sys/class/power_supply/%s/status", n);
		l_gets(p, st, sizeof st);
		at += snprintf(line + at, sizeof line - (size_t)at, "%s%s%s%s%s%s", at ? ", " : "", n, cap[0] ? " " : "",
			       cap, cap[0] ? "%" : "", st[0] ? "" : "");
		if (st[0])
			at += snprintf(line + at, sizeof line - (size_t)at, " %s", st);
	}
	if (at)
		printf("linuxdrv: power: %s\n", line);
	l_list("/sys/class/net", names, sizeof names);
	printf("linuxdrv: network: %s\n", names[0] ? names : "none");
	l_list("/sys/class/sound", names, sizeof names);
	printf("linuxdrv: sound: %s\n", names[0] ? names : "none");
	l_list("/sys/bus/iio/devices", names, sizeof names);
	if (names[0])
		printf("linuxdrv: sensors: %s\n", names);
	l_list("/sys/class/input", names, sizeof names);
	printf("linuxdrv: input: %s\n", names);
	l_cat("/sys/kernel/debug/devices_deferred", "linuxdrv: still waiting: ");
}

static void *bring_up(void *arg)
{
	(void)arg;
	/* after the phone shell is up (it must not wait for this) */
	for (int t = 0; t < 240; t++) {
		FILE *f = fopen("/tmp/flare-ui.log", "r");
		char l[256];
		int up = 0;
		while (f && fgets(l, sizeof l, f))
			up |= strstr(l, "flarehome: ready") != NULL;
		if (f)
			fclose(f);
		if (up)
			break;
		sleep(1);
	}
	l_sys(NR_MOUNT, (long)"proc", (long)"/proc", (long)"proc", 0, 0);
	mount_partitions();
	firmware();
	FILE *st = fopen("/etc/linuxdrv/stages", "r");
	char line[1024];
	while (st && fgets(line, sizeof line, st)) {
		char *save = NULL, *name = strtok_r(line, " \n", &save);
		if (!name)
			continue;
		int n = 0;
		for (char *c = strtok_r(NULL, " \n", &save); c; c = strtok_r(NULL, " \n", &save))
			n += unhide(c);
		printf("linuxdrv: [%.1f s] stage %s: %d devices given to Linux\n", since_boot(), name, n);
		fflush(stdout);
		static char bound[2048];
		int total = 0;
		for (int t = 0; t < 10; t++) {
			int k = probe_unbound(bound, sizeof bound);
			if (k)
				printf("linuxdrv: stage %s: %s\n", name, bound);
			total += k;
			if (!k && t >= 2)
				break;
			usleep(500000);
		}
		start_remoteprocs();
		fflush(stdout);
	}
	if (st)
		fclose(st);
	gpu_check();
	summary();
	printf("linuxdrv: [%.1f s] bring-up done\n", since_boot());
	fflush(stdout);
	return NULL;
}

/* ---- Linux's files and sockets for Flare's programs (kernel/src/linux/remote.rs) ----
 * The Flare kernel queues each system call a Flare program makes on one of
 * them as a request on /dev/flarelk; this makes the same call here, on this
 * Linux, in a thread of its own (calls block: recv, accept, a sound card's
 * write), and writes the reply back. A poller reports each descriptor's
 * readiness, for poll, epoll and select on the Flare side. */
#define NR_FCNTL 25
#define NR_LSEEK 62
#define NR_PPOLL 73
#define NR_PWRITE64 68
#define NR_FSTAT 80
#define NR_NEWFSTATAT 79
#define NR_FACCESSAT 48
#define NR_READLINKAT 78
#define NR_GETDENTS64 61
#define NR_SOCKET 198
#define NR_BIND 200
#define NR_LISTEN 201
#define NR_CONNECT 203
#define NR_GETSOCKNAME 204
#define NR_GETPEERNAME 205
#define NR_SENDTO 206
#define NR_RECVFROM 207
#define NR_SETSOCKOPT 208
#define NR_GETSOCKOPT 209
#define NR_SHUTDOWN 210
#define NR_SENDMSG 211
#define NR_RECVMSG 212
#define NR_ACCEPT4 242

enum { OP_OPEN = 1, OP_CLOSE, OP_READ, OP_WRITE, OP_IOCTL, OP_FSTAT, OP_LSEEK, OP_GETDENTS, OP_SOCKET, OP_CONNECT,
       OP_BIND, OP_LISTEN, OP_ACCEPT, OP_SENDTO, OP_RECVFROM, OP_GETSOCKOPT, OP_SETSOCKOPT, OP_SOCKNAME, OP_SHUTDOWN,
       OP_SETFL, OP_STAT, OP_ACCESS, OP_READLINK };

struct lk_req {
	uint64_t id;
	uint32_t op;
	int32_t handle;
	uint64_t a[3];
	uint32_t len, pad;
	unsigned char data[];
};

static int lk = -1;
static pthread_mutex_t lk_wlock = PTHREAD_MUTEX_INITIALIZER;

/* the descriptors Flare's programs hold, for the poller */
#define LK_MAX 1024
static struct { int fd; short last; } lk_fds[LK_MAX];
static int lk_nfds;
static pthread_mutex_t lk_fdlock = PTHREAD_MUTEX_INITIALIZER;

static void lk_reply(uint64_t id, int64_t res, const void *data, uint32_t len, uint32_t kind)
{
	unsigned char hdr[24];
	memcpy(hdr, &id, 8);
	memcpy(hdr + 8, &res, 8);
	memcpy(hdr + 16, &len, 4);
	memcpy(hdr + 20, &kind, 4);
	pthread_mutex_lock(&lk_wlock);
	if (len) {
		unsigned char *b = malloc(24 + len);
		if (b) {
			memcpy(b, hdr, 24);
			memcpy(b + 24, data, len);
			if (write(lk, b, 24 + len) < 0)
				perror("linuxdrv: flarelk");
			free(b);
		}
	} else if (write(lk, hdr, 24) < 0) {
		perror("linuxdrv: flarelk");
	}
	pthread_mutex_unlock(&lk_wlock);
}

static void lk_track(int fd, int add)
{
	pthread_mutex_lock(&lk_fdlock);
	if (add && lk_nfds < LK_MAX) {
		lk_fds[lk_nfds].fd = fd;
		lk_fds[lk_nfds].last = -1;
		lk_nfds++;
	} else if (!add) {
		for (int i = 0; i < lk_nfds; i++)
			if (lk_fds[i].fd == fd) {
				lk_fds[i] = lk_fds[--lk_nfds];
				break;
			}
	}
	pthread_mutex_unlock(&lk_fdlock);
}

static void *lk_poller(void *arg)
{
	(void)arg;
	static struct { int fd; short events, revents; } pf[LK_MAX];
	for (;;) {
		pthread_mutex_lock(&lk_fdlock);
		int n = lk_nfds;
		for (int i = 0; i < n; i++) {
			pf[i].fd = lk_fds[i].fd;
			pf[i].events = 0x1 | 0x4 | 0x2; /* POLLIN | POLLOUT | POLLPRI */
			pf[i].revents = 0;
		}
		pthread_mutex_unlock(&lk_fdlock);
		struct { long s, ns; } ts = { 0, 30000000 };
		long r = n ? l_sys(NR_PPOLL, (long)pf, n, (long)&ts, 0, 0) : 0;
		if (!n || r < 0)
			usleep(30000);
		pthread_mutex_lock(&lk_fdlock);
		for (int i = 0; i < n && i < lk_nfds; i++) {
			if (lk_fds[i].fd != pf[i].fd || lk_fds[i].last == pf[i].revents)
				continue;
			lk_fds[i].last = pf[i].revents;
			lk_reply((uint64_t)pf[i].fd, (uint16_t)pf[i].revents, NULL, 0, 1);
		}
		pthread_mutex_unlock(&lk_fdlock);
		/* (readiness without a change waits for the next round) */
		if (r == 0 && n)
			continue;
		usleep(5000);
	}
	return NULL;
}

static void *lk_serve(void *arg)
{
	struct lk_req *q = arg;
	long h = q->handle, r = -38 /* ENOSYS */;
	unsigned char *out = NULL;
	uint32_t olen = 0;
	switch (q->op) {
	case OP_OPEN:
		r = l_sys(NR_OPENAT, AT_FDCWD, (long)q->data, (long)q->a[0], (long)q->a[1], 0);
		if (r >= 0)
			lk_track((int)r, 1);
		break;
	case OP_CLOSE:
		lk_track((int)h, 0);
		l_close((int)h);
		free(q);
		return NULL; /* no reply */
	case OP_READ:
	case OP_GETDENTS:
		out = malloc(q->a[0] + 1);
		if (q->op == OP_GETDENTS)
			r = l_sys(NR_GETDENTS64, h, (long)out, (long)q->a[0], 0, 0);
		else if (q->a[1] != UINT64_MAX)
			r = l_sys(NR_PREAD64, h, (long)out, (long)q->a[0], (long)q->a[1], 0);
		else
			r = l_sys(NR_READ, h, (long)out, (long)q->a[0], 0, 0);
		olen = r > 0 ? (uint32_t)r : 0;
		break;
	case OP_WRITE:
		if (q->a[1] != UINT64_MAX)
			r = l_sys(NR_PWRITE64, h, (long)q->data, q->len, (long)q->a[1], 0);
		else
			r = l_sys(NR_WRITE, h, (long)q->data, q->len, 0, 0);
		break;
	case OP_IOCTL: {
		uint64_t cmd = q->a[0], outb = q->a[2] & 0xffffffff, frames = q->a[2] >> 32;
		if (cmd == 0x40184150 || cmd == 0x80184151) {
			/* ALSA frames: struct snd_xferi pointing at our copy; the
			 * reply is the frames moved (i64), then (reads) the frames */
			uint64_t fb = q->a[1] ? q->a[1] : 1;
			out = malloc(8 + (cmd == 0x80184151 ? frames * fb : 0));
			struct { int64_t result; void *buf; uint64_t frames; } x = { 0, cmd == 0x80184151 ? out + 8 : q->data, frames };
			r = l_sys(NR_IOCTL, h, (long)cmd, (long)&x, 0, 0);
			memcpy(out, &x.result, 8);
			olen = 8;
			if (cmd == 0x80184151 && r == 0 && x.result > 0)
				olen = 8 + (uint32_t)((uint64_t)x.result * fb);
			break;
		}
		if (outb || q->len) {
			size_t sz = outb > q->len ? outb : q->len;
			out = calloc(1, sz ? sz : 1);
			memcpy(out, q->data, q->len);
			r = l_sys(NR_IOCTL, h, (long)cmd, (long)out, 0, 0);
			olen = (uint32_t)outb;
		} else {
			r = l_sys(NR_IOCTL, h, (long)cmd, (long)q->a[1], 0, 0);
		}
		break;
	}
	case OP_FSTAT:
		out = calloc(1, 128);
		r = l_sys(NR_FSTAT, h, (long)out, 0, 0, 0);
		olen = r == 0 ? 128 : 0;
		break;
	case OP_STAT:
		out = calloc(1, 128);
		r = l_sys(NR_NEWFSTATAT, AT_FDCWD, (long)q->data, (long)out, (long)q->a[0], 0);
		olen = r == 0 ? 128 : 0;
		break;
	case OP_ACCESS:
		r = l_sys(NR_FACCESSAT, AT_FDCWD, (long)q->data, (long)q->a[0], 0, 0);
		break;
	case OP_READLINK:
		out = malloc(q->a[0] + 1);
		r = l_sys(NR_READLINKAT, AT_FDCWD, (long)q->data, (long)out, (long)q->a[0], 0);
		olen = r > 0 ? (uint32_t)r : 0;
		break;
	case OP_LSEEK:
		r = l_sys(NR_LSEEK, h, (long)q->a[0], (long)q->a[1], 0, 0);
		break;
	case OP_SOCKET:
		r = l_sys(NR_SOCKET, (long)q->a[0], (long)q->a[1], (long)q->a[2], 0, 0);
		if (r >= 0)
			lk_track((int)r, 1);
		break;
	case OP_CONNECT:
		r = l_sys(NR_CONNECT, h, (long)q->data, q->len, 0, 0);
		break;
	case OP_BIND:
		r = l_sys(NR_BIND, h, (long)q->data, q->len, 0, 0);
		break;
	case OP_LISTEN:
		r = l_sys(NR_LISTEN, h, (long)q->a[0], 0, 0, 0);
		break;
	case OP_SHUTDOWN:
		r = l_sys(NR_SHUTDOWN, h, (long)q->a[0], 0, 0, 0);
		break;
	case OP_SETFL:
		r = l_sys(NR_FCNTL, h, 4 /* F_SETFL */, (long)q->a[0], 0, 0);
		break;
	case OP_ACCEPT: {
		unsigned char addr[128];
		uint32_t al = sizeof addr;
		r = l_sys(NR_ACCEPT4, h, (long)addr, (long)&al, (long)q->a[0], 0);
		if (r >= 0) {
			lk_track((int)r, 1);
			out = malloc(al);
			memcpy(out, addr, al);
			olen = al;
		}
		break;
	}
	case OP_SOCKNAME: {
		unsigned char addr[128];
		uint32_t al = sizeof addr;
		r = l_sys(q->a[0] ? NR_GETPEERNAME : NR_GETSOCKNAME, h, (long)addr, (long)&al, 0, 0);
		if (r == 0) {
			out = malloc(al);
			memcpy(out, addr, al);
			olen = al;
		}
		break;
	}
	case OP_SENDTO: {
		uint64_t len = q->a[0], alen = q->a[2];
		r = l_sys6(NR_SENDTO, h, (long)q->data, (long)len, (long)q->a[1], alen ? (long)(q->data + len) : 0,
			   (long)alen);
		break;
	}
	case OP_RECVFROM: {
		/* reply: u32 address length, the address, (recvmsg: u32 flags),
		 * the bytes */
		uint64_t len = q->a[0];
		out = malloc(4 + 128 + 4 + len);
		unsigned char addr[128];
		uint32_t al = sizeof addr, mflags = 0;
		unsigned char *body = out + 4 + 128 + 4;
		if (q->a[2] == 1) {
			struct { void *b; size_t l; } iov = { body, len };
			struct { void *name; uint32_t namelen, p0; void *iov; size_t iovlen; void *ctl; size_t ctllen; int flags, p1; } m =
				{ addr, sizeof addr, 0, &iov, 1, NULL, 0, 0, 0 };
			r = l_sys(NR_RECVMSG, h, (long)&m, (long)q->a[1], 0, 0);
			al = m.namelen;
			mflags = (uint32_t)m.flags;
		} else {
			r = l_sys6(NR_RECVFROM, h, (long)body, (long)len, (long)q->a[1], (long)addr, (long)&al);
		}
		if (r < 0)
			break;
		if (al > sizeof addr)
			al = sizeof addr;
		/* packed: [alen][addr][flags?][bytes] */
		unsigned char *p = out;
		memcpy(p, &al, 4);
		memcpy(p + 4, addr, al);
		uint32_t at = 4 + al;
		if (q->a[2] == 1) {
			memcpy(p + at, &mflags, 4);
			at += 4;
		}
		memmove(p + at, body, (size_t)r);
		olen = at + (uint32_t)r;
		break;
	}
	case OP_GETSOCKOPT: {
		uint32_t ol = (uint32_t)q->a[2];
		out = calloc(1, ol + 1);
		r = l_sys(NR_GETSOCKOPT, h, (long)q->a[0], (long)q->a[1], (long)out, (long)&ol);
		olen = r == 0 ? ol : 0;
		break;
	}
	case OP_SETSOCKOPT:
		if (q->a[0] == 1 && q->a[1] == 26 /* SO_ATTACH_FILTER */ && q->len >= 2) {
			struct { uint16_t len; uint16_t p[3]; void *filter; } prog;
			memcpy(&prog.len, q->data, 2);
			prog.filter = q->data + 2;
			r = l_sys(NR_SETSOCKOPT, h, 1, 26, (long)&prog, sizeof prog);
		} else {
			r = l_sys(NR_SETSOCKOPT, h, (long)q->a[0], (long)q->a[1], (long)q->data, q->len);
		}
		break;
	}
	lk_reply(q->id, r, out, olen, 0);
	free(out);
	free(q);
	return NULL;
}

static void *lk_dispatch(void *arg)
{
	(void)arg;
	static unsigned char buf[(1 << 20) + 4096];
	for (;;) {
		long n = read(lk, buf, sizeof buf);
		if (n < (long)sizeof(struct lk_req)) {
			if (n < 0 && errno != EINTR && errno != EAGAIN) {
				perror("linuxdrv: flarelk");
				return NULL;
			}
			continue;
		}
		struct lk_req *q = malloc((size_t)n + 1);
		memcpy(q, buf, (size_t)n);
		((unsigned char *)q)[n] = 0; /* paths end */
		pthread_t t;
		pthread_attr_t at;
		pthread_attr_init(&at);
		pthread_attr_setstacksize(&at, 256 << 10);
		pthread_attr_setdetachstate(&at, PTHREAD_CREATE_DETACHED);
		if (pthread_create(&t, &at, lk_serve, q))
			lk_serve(q);
		pthread_attr_destroy(&at);
	}
}

static void serve_flare(void)
{
	lk = open("/dev/flarelk", O_RDWR);
	if (lk < 0)
		return;
	pthread_t a, b;
	pthread_create(&a, NULL, lk_dispatch, NULL);
	pthread_create(&b, NULL, lk_poller, NULL);
	printf("linuxdrv: Linux's files and sockets are Flare's (/linux, /sys/class, /dev/snd, network)\n");
}

int main(int argc, char **argv)
{
	if (argc < 2) {
		fprintf(stderr, "usage: linuxdrv BOARD.dtb [Linux kernel arguments...]\n");
		return 2;
	}
	setvbuf(stdout, NULL, _IOLBF, 0);
	hw = open("/dev/flarehw", O_RDWR);
	if (hw < 0) {
		fprintf(stderr, "linuxdrv: /dev/flarehw: %s (the composition gives Linux no hardware)\n", strerror(errno));
		return 1;
	}
	int ndevs = 0;
	for (int i = 0; nwin < MAXWIN; i++) {
		struct hw_info in = { .index = (uint32_t)i };
		if (ioctl(hw, FLAREHW_INFO, &in))
			break;
		for (uint32_t r = 0; r < in.nregs && nwin < MAXWIN; r++) {
			win[nwin].base = in.regs[r][0];
			win[nwin].size = in.regs[r][1];
			memcpy(win[nwin].name, in.name, sizeof in.name);
			nwin++;
		}
		ndevs++;
	}
	printf("linuxdrv: %d devices from Flare (%d register windows)\n", ndevs, nwin);

	/* the device tree, which the kernel keeps for good */
	int fd = open(argv[1], O_RDONLY);
	struct stat st;
	if (fd < 0 || fstat(fd, &st)) {
		fprintf(stderr, "linuxdrv: %s: %s\n", argv[1], strerror(errno));
		return 1;
	}
	void *fdt = malloc((size_t)st.st_size);
	if (read(fd, fdt, (size_t)st.st_size) != st.st_size)
		return 1;
	close(fd);
	lkl_qrt_fdt = fdt;
	dtb = fdt;
	dtb_len = (size_t)st.st_size;

	/* coherent DMA memory: non-cacheable, as the SoC's DMA does not snoop */
	pool_size = 4 << 20;
	if (dma_alloc(pool_size, &pool_phys))
		return 1;
	pool_va = mmap(NULL, pool_size, PROT_READ | PROT_WRITE, MAP_SHARED, hw, (off_t)(pool_phys | FLAREHW_NOCACHE));
	if (pool_va == MAP_FAILED)
		return 1;
	lkl_qrt_dma_base = pool_phys;
	lkl_qrt_dma_size = pool_size;

	char cmd[512] = "mem=160M loglevel=6 log_buf_len=1M"; /* QRT gives Linux 160 MiB on the Mi A1 */
	for (int i = 2; i < argc; i++) {
		strncat(cmd, " ", sizeof cmd - strlen(cmd) - 1);
		strncat(cmd, argv[i], sizeof cmd - strlen(cmd) - 1);
	}
	pthread_t irq, wd;
	pthread_create(&irq, NULL, irq_thread, NULL);
	pthread_create(&wd, NULL, watchdog, NULL);
	if (lkl_init(&ops) < 0 || lkl_start_kernel("%s", cmd) < 0) {
		fprintf(stderr, "linuxdrv: Linux did not start\n");
		return 1;
	}
	printf("linuxdrv: [%.2f s] Linux is up (RAM %llu MiB at %#llx, coherent pool %#llx)\n",
	       since_boot(), (unsigned long long)(ram_size >> 20), (unsigned long long)ram_phys,
	       (unsigned long long)pool_phys);
	l_sys(NR_MKDIRAT, AT_FDCWD, (long)"/dev", 0755, 0, 0);
	l_sys(NR_MOUNT, (long)"devtmpfs", (long)"/dev", (long)"devtmpfs", 0, 0);
	l_sys(NR_MKDIRAT, AT_FDCWD, (long)"/sys", 0755, 0, 0);
	l_sys(NR_MOUNT, (long)"sysfs", (long)"/sys", (long)"sysfs", 0, 0);
	l_sys(NR_MOUNT, (long)"debugfs", (long)"/sys/kernel/debug", (long)"debugfs", 0, 0);
	l_sys(NR_MKDIRAT, AT_FDCWD, (long)"/proc", 0555, 0, 0);
	l_sys(NR_MOUNT, (long)"proc", (long)"/proc", (long)"proc", 0, 0);
	report();
	serve_flare();
	pthread_t up;
	pthread_create(&up, NULL, bring_up, NULL);
	/* keep the drivers running */
	for (;;)
		pause();
}
