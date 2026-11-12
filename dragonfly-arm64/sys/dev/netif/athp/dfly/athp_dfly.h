/*
 * FreeBSD -> DragonFly glue for athp, force-included before every file.
 * The DragonFly headers are pulled in first so the macros below only
 * rename athp's uses, never the system's own definitions.
 */
#ifndef _ATHP_DFLY_H_
#define	_ATHP_DFLY_H_

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/malloc.h>
#include <sys/lock.h>
#include <sys/mutex.h>
#include <sys/mutex2.h>
#include <sys/condvar.h>
#include <sys/callout.h>
#include <sys/taskqueue.h>
#include <sys/bus.h>
#include <sys/rman.h>
#include <sys/mbuf.h>
#include <sys/socket.h>
#include <sys/sockio.h>
#include <sys/endian.h>
#include <sys/firmware.h>
#include <sys/module.h>
#include <sys/conf.h>
#include <sys/proc.h>
#include <sys/sysctl.h>
#include <sys/thread2.h>
#include <sys/limits.h>
#include <sys/queue.h>
#include <sys/bus_dma.h>
#include <net/if.h>
#include <net/if_var.h>
#include <net/if_media.h>
#include <net/ifq_var.h>

/* ---- mutexes: a lockmgr lock in a wrapper ---- */
struct athp_mtx {
	struct lock	lk;
};
#define	mtx			athp_mtx
#define	MTX_DEF			0
#define	MTX_RECURSE		1
#define	MTX_NETWORK_LOCK	"athp network"
#define	MA_OWNED		1
#define	MA_NOTOWNED		2
#define	mtx_init(m, name, type, opts)					\
	lockinit(&(m)->lk, (name), 0, ((opts) & MTX_RECURSE) ? LK_CANRECURSE : 0)
#define	mtx_destroy(m)		lockuninit(&(m)->lk)
#define	mtx_lock(m)		lockmgr(&(m)->lk, LK_EXCLUSIVE)
#define	mtx_unlock(m)		lockmgr(&(m)->lk, LK_RELEASE)
#define	mtx_owned(m)		(lockstatus(&(m)->lk, curthread) == LK_EXCLUSIVE)
#define	mtx_assert(m, what)	do { } while (0)
#define	mtx_initialized(m)	1
#define	mtx_sleep(chan, m, pri, wmesg, timo)				\
	lksleep((chan), &(m)->lk, ((pri) & PCATCH), (wmesg), (timo))
#define	msleep(chan, m, pri, wmesg, timo) mtx_sleep(chan, m, pri, wmesg, timo)
#define	callout_init_mtx(c, m, f)	callout_init_lk((c), &(m)->lk)
#define	callout_init(c, mpsafe)		callout_init_mp(c)

/* ---- task queues ---- */
#define	PI_NET			TDPRI_KERN_DAEMON
#undef	taskqueue_start_threads
#define	taskqueue_start_threads(tqp, n, pri, ...)			\
	taskqueue_start_threads((tqp), (n), TDPRI_KERN_DAEMON, -1, __VA_ARGS__)
#define	taskqueue_drain_all(tq)		do { } while (0)

/* ---- misc kernel API ---- */
#define	M_NOWAIT_ATHP		M_NOWAIT
#ifndef	nitems
#define	nitems(x)		(sizeof(x) / sizeof((x)[0]))
#endif
#define	pause(wmesg, timo)	tsleep(&(timo), 0, (wmesg), (timo) > 0 ? (timo) : 1)
#define	DELAY(x)		DELAY(x)
#define	device_printf		device_printf
#define	bus_get_dma_tag(dev)	NULL
#define	ticks_athp		ticks


/* ---- libkern spellings ---- */
#define	printf			kprintf
#define	snprintf		ksnprintf
#define	vsnprintf		kvsnprintf
#define	vprintf			kvprintf
#define	sprintf			ksprintf
#define	malloc(sz, type, fl)	kmalloc((sz), (type), (fl))
#define	free(p, type)		kfree((p), (type))
#define	strdup(s, type)		kstrdup((s), (type))
#define	MPASS(x)		KKASSERT(x)
#define	KASSERT_ATHP(x, m)	KASSERT(x, m)
static __inline unsigned long
roundup_pow_of_two(unsigned long n)
{
	unsigned long r = 1;
	while (r < n)
		r <<= 1;
	return (r);
}
struct mbuf	*athp_dfly_m_getm2(int len, int how, int type, int flags);
#define	m_getm2(m, len, how, type, flags) athp_dfly_m_getm2((len), (how), (type), (flags))


/* ---- mbuf ---- */
#ifndef	M_START
#define	M_START(m)							\
	(((m)->m_flags & M_EXT) ? (m)->m_ext.ext_buf :			\
	 ((m)->m_flags & M_PKTHDR) ? &(m)->m_pktdat[0] : &(m)->m_dat[0])
#define	M_SIZE(m)							\
	(((m)->m_flags & M_EXT) ? (int)(m)->m_ext.ext_size :		\
	 ((m)->m_flags & M_PKTHDR) ? MHLEN : MLEN)
#endif

/* ---- linuxisms not covered by hal/linux_compat.h ---- */
#define	mb()			__asm __volatile("dmb sy" ::: "memory")
static __inline int
ilog2(unsigned long v)
{
	int r = -1;
	while (v) {
		v >>= 1;
		r++;
	}
	return (r);
}
#define	PRI_USER		0
#define	kern_yield(p)		lwkt_yield()
#define	PCIER_LINK_CTL		PCIER_LINKCTRL

/* ---- net80211 bits newer than DragonFly's copy ---- */
#define	IEEE80211_F_SURVEY_TIME		0x00000001
#define	IEEE80211_F_SURVEY_TIME_BUSY	0x00000002
#define	IEEE80211_F_SURVEY_NOISE_DBM	0x00000004
#define	IEEE80211_F_SURVEY_TSC		0x00000008
#define	IEEE80211_F_SURVEY_TIME_RX	0x00000010
#define	IEEE80211_F_SURVEY_TIME_TX	0x00000020
struct ieee80211_channel_survey {
	uint32_t	s_flags;
	uint32_t	s_time;
	uint32_t	s_time_busy;
	uint32_t	s_time_rx;
	uint32_t	s_time_tx;
	int32_t		s_noise;
};


/* ---- queue.h ---- */
#ifndef	TAILQ_FOREACH_SAFE
#define	TAILQ_FOREACH_SAFE	TAILQ_FOREACH_MUTABLE
#endif
#ifndef	LIST_FOREACH_SAFE
#define	LIST_FOREACH_SAFE	LIST_FOREACH_MUTABLE
#endif
#ifndef	STAILQ_FOREACH_SAFE
#define	STAILQ_FOREACH_SAFE	STAILQ_FOREACH_MUTABLE
#endif

/* ---- bus_dma: no filter and no lock callbacks here ---- */
#define	bus_dma_tag_create(parent, align, bound, low, high, filt, filtarg, \
	    maxsz, nseg, maxsegsz, fl, lockf, locka, tagp)		\
	bus_dma_tag_create((parent), (align), (bound), (low), (high),	\
	    (maxsz), (nseg), (maxsegsz), (fl), (tagp))
#define	bus_dmamap_load_mbuf_sg(tag, map, m, segs, nsegp, fl)		\
	bus_dmamap_load_mbuf_segment((tag), (map), (m), (segs),		\
	    (int)nitems(segs), (nsegp), (fl))

/* ---- network epochs do not exist; the wlan serializer covers it ---- */
struct epoch_tracker {
	int	et_unused;
};
#define	NET_EPOCH_ENTER(et)	do { (void)&(et); } while (0)
#define	NET_EPOCH_EXIT(et)	do { } while (0)

/* ---- sysctl ---- */
#undef	SYSCTL_ADD_UQUAD
#define	SYSCTL_ADD_UQUAD	SYSCTL_ADD_U64
#ifndef	CTLFLAG_RWTUN
#define	CTLFLAG_RWTUN		CTLFLAG_RW
#endif

#define	ffsll(x)		__builtin_ffsll(x)

static __inline void
mbufq_concat(struct mbufq *to, struct mbufq *from)
{
	struct mbuf *m;

	while ((m = mbufq_dequeue(from)) != NULL)
		mbufq_enqueue(to, m);
}

/* PCIe link control: ASPM L0s|L1 */
#define	PCIEM_LINK_CTL_ASPMC	0x0003


/* ---- interrupt filters (FreeBSD) ---- */
#define	FILTER_STRAY		0x01
#define	FILTER_HANDLED		0x02
#define	FILTER_SCHEDULE_THREAD	0x04
#define	INTR_TYPE_NET		0


/* progress for the shell, in hw.mi5.wlan (if_athp_dfly.c) */
void	athp_dfly_status(const char *fmt, ...) __printflike(1, 2);
void	athp_dfly_note(const char *fmt, ...) __printflike(1, 2);
extern volatile int athp_dfly_in_reset;	/* chip resetting: BAR off limits */


/* athp brings its own idr; keep it apart from the kernel's (drm) one */
#define	ida_destroy	athp_ida_destroy
#define	ida_get_new_above	athp_ida_get_new_above
#define	ida_init	athp_ida_init
#define	ida_pre_get	athp_ida_pre_get
#define	ida_remove	athp_ida_remove
#define	ida_simple_get	athp_ida_simple_get
#define	ida_simple_remove	athp_ida_simple_remove
#define	idr_alloc	athp_idr_alloc
#define	idr_alloc_cyclic	athp_idr_alloc_cyclic
#define	idr_alloc_locked	athp_idr_alloc_locked
#define	idr_destroy	athp_idr_destroy
#define	idr_find	athp_idr_find
#define	idr_find_layer_locked	athp_idr_find_layer_locked
#define	idr_find_locked	athp_idr_find_locked
#define	idr_for_each	athp_idr_for_each
#define	idr_for_each_layer	athp_idr_for_each_layer
#define	idr_get	athp_idr_get
#define	idr_get_new	athp_idr_get_new
#define	idr_get_new_above	athp_idr_get_new_above
#define	idr_get_new_above_locked	athp_idr_get_new_above_locked
#define	idr_get_new_locked	athp_idr_get_new_locked
#define	idr_get_next	athp_idr_get_next
#define	idr_init	athp_idr_init
#define	idr_max	athp_idr_max
#define	idr_pos	athp_idr_pos
#define	idr_pre_get	athp_idr_pre_get
#define	idr_remove	athp_idr_remove
#define	idr_remove_all	athp_idr_remove_all
#define	idr_remove_layer	athp_idr_remove_layer
#define	idr_remove_locked	athp_idr_remove_locked
#define	idr_replace	athp_idr_replace

#endif
