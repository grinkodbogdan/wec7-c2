// SPDX-License-Identifier: GPL-2.0-only
/*
 * /dev/dri/card0 and /dev/dri/renderD128 for the Adreno 530: the subset
 * of the DRM and drm/msm interface that Mesa's freedreno uses (libdrm's
 * drmGetVersion, GEM_CLOSE, and the msm ioctls of include/uapi/drm/
 * msm_drm.h at interface version 1.5), so that freedreno runs unchanged.
 *
 * Modelled on Linux's drivers/gpu/drm/msm (msm_drv.c, msm_gem.c,
 * msm_gem_submit.c, msm_submitqueue.c); GPL-2.0, as the rest of this
 * directory.  Copyright (c) 2026 The DragonFly Project.
 *
 * Simplifications, for now:
 *  - buffers come from one physically contiguous pool reserved at boot,
 *    and the GPU sees physical addresses (its SMMU stream is bypassed),
 *    so a buffer's iova is its physical address and so is its mmap
 *    offset;
 *  - the GPU is not cache coherent: user mappings are cached, and the
 *    kernel cleans and invalidates every buffer of a submission before
 *    the GPU runs it, and invalidates a buffer in CPU_PREP;
 *  - fences are polled (the ring's CACHE_FLUSH_TS writes), no interrupts;
 *  - one ring, no preemption, no fence fds or syncobjs.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/conf.h>
#include <sys/device.h>
#include <sys/devfs.h>
#include <sys/file.h>
#include <sys/malloc.h>
#include <sys/lock.h>
#include <sys/sysctl.h>
#include <sys/uio.h>

#include <vm/vm.h>
#include <vm/pmap.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#include "uapi/msm_drm.h"
#include "adreno_a5xx_dfly.h"

#define	MSM_VERSION_MAJOR	1
#define	MSM_VERSION_MINOR	5	/* robustness; not VA_SIZE (9) */
#define	MSM_VERSION_PATCH	0

#define	POOL_SIZE		(256UL * 1024 * 1024)
#define	MAXHANDLES		8192

MALLOC_DEFINE(M_MSMDRM, "msmdrm", "msm DRM device");

struct gbo {
	int		refs;
	vm_paddr_t	pa;
	size_t		size;		/* bytes, page multiple */
	uint32_t	fence;		/* last submission using it */
};

struct mfile {
	struct gbo	*h[MAXHANDLES];	/* handle n is h[n], 0 unused */
	uint32_t	nextq;
};

static struct {
	vm_paddr_t	pa;
	uint8_t		*kva;
	size_t		npages;
	uint8_t		*used;		/* one byte per page: simple, enough */
	struct lock	lk;
	uint32_t	seqno;
	int		up;
} msm;

static d_open_t		msm_open;
static d_ioctl_t	msm_ioctl;
static d_mmap_t		msm_mmap;

static struct dev_ops msm_ops = {
	{ "drm", 0, D_MPSAFE },
	.d_open =	msm_open,
	.d_ioctl =	msm_ioctl,
	.d_mmap =	msm_mmap,
};

/* ---- caches: the GPU does not snoop the cpu's ---- */

static void
cache_wbinv(vm_paddr_t pa, size_t len)
{
	vm_offset_t a = (vm_offset_t)PHYS_TO_DMAP(pa) & ~63UL;
	vm_offset_t end = (vm_offset_t)PHYS_TO_DMAP(pa) + len;

	for (; a < end; a += 64)
		__asm __volatile("dc civac, %0" : : "r" (a) : "memory");
	__asm __volatile("dsb sy" : : : "memory");
}

/* ---- the pool ---- */

static struct gbo *
gbo_alloc(size_t size)
{
	size_t n = round_page(size) / PAGE_SIZE, i, run = 0;
	struct gbo *bo;

	if (n == 0 || n > msm.npages)
		return (NULL);
	for (i = 0; i < msm.npages; ++i) {
		run = msm.used[i] ? 0 : run + 1;
		if (run == n)
			break;
	}
	if (run != n)
		return (NULL);
	i = i + 1 - n;
	memset(msm.used + i, 1, n);
	bo = kmalloc(sizeof(*bo), M_MSMDRM, M_WAITOK | M_ZERO);
	bo->refs = 1;
	bo->pa = msm.pa + i * PAGE_SIZE;
	bo->size = n * PAGE_SIZE;
	memset((void *)PHYS_TO_DMAP(bo->pa), 0, bo->size);
	cache_wbinv(bo->pa, bo->size);
	return (bo);
}

static void
gbo_unref(struct gbo *bo)
{
	if (--bo->refs > 0)
		return;
	memset(msm.used + (bo->pa - msm.pa) / PAGE_SIZE, 0,
	    bo->size / PAGE_SIZE);
	kfree(bo, M_MSMDRM);
}

/* ---- fences ---- */

static int
fence_done(uint32_t f)
{
	return ((int32_t)(a5xx_dfly_fence() - f) >= 0);
}

static int
fence_wait(uint32_t f, int ticks)
{
	while (!fence_done(f)) {
		if (ticks-- <= 0)
			return (ETIMEDOUT);
		tsleep(&msm, 0, "gpufnc", 1);
	}
	return (0);
}

/* ---- per open ---- */

static void
mfile_dtor(void *arg)
{
	struct mfile *mf = arg;
	int i;

	lockmgr(&msm.lk, LK_EXCLUSIVE);
	for (i = 1; i < MAXHANDLES; ++i)
		if (mf->h[i] != NULL)
			gbo_unref(mf->h[i]);
	lockmgr(&msm.lk, LK_RELEASE);
	kfree(mf, M_MSMDRM);
}

static int
msm_open(struct dev_open_args *ap)
{
	struct mfile *mf;
	int error;

	if (!msm.up)
		return (ENXIO);
	mf = kmalloc(sizeof(*mf), M_MSMDRM, M_WAITOK | M_ZERO);
	mf->nextq = 1;
	error = devfs_set_cdevpriv(ap->a_fpp ? *ap->a_fpp : NULL, mf,
	    mfile_dtor);
	if (error)
		kfree(mf, M_MSMDRM);
	return (error);
}

static struct gbo *
lookup(struct mfile *mf, uint32_t handle)
{
	if (handle == 0 || handle >= MAXHANDLES)
		return (NULL);
	return (mf->h[handle]);
}

static int
new_handle(struct mfile *mf, struct gbo *bo, uint32_t *handle)
{
	int i;

	for (i = 1; i < MAXHANDLES; ++i)
		if (mf->h[i] == NULL) {
			mf->h[i] = bo;
			*handle = i;
			return (0);
		}
	return (ENOSPC);
}

static int
copyout_str(const char *s, char *uptr, __kernel_size_t *len)
{
	size_t n = strlen(s);
	int error = 0;

	if (uptr != NULL && *len > 0)
		error = copyout(s, uptr, MIN(n, *len));
	*len = n;
	return (error);
}

/* ---- the msm ioctls ---- */

static int
get_param(struct drm_msm_param *p)
{
	if (p->pipe != MSM_PIPE_3D0)
		return (EINVAL);
	switch (p->param) {
	case MSM_PARAM_GPU_ID:		p->value = 530; break;
	case MSM_PARAM_CHIP_ID:		p->value = 0x05030002; break;
	case MSM_PARAM_GMEM_SIZE:	p->value = 1024 * 1024; break;
	case MSM_PARAM_GMEM_BASE:	p->value = 0x100000; break;
	case MSM_PARAM_MAX_FREQ:	p->value = 624000000; break;
	case MSM_PARAM_TIMESTAMP:	p->value = a5xx_dfly_timestamp(); break;
	case MSM_PARAM_PRIORITIES:	p->value = 1; break;
	case MSM_PARAM_PP_PGTABLE:	p->value = 0; break;
	case MSM_PARAM_FAULTS:		p->value = 0; break;
	case MSM_PARAM_SUSPENDS:	p->value = 0; break;
	case MSM_PARAM_VA_START:	p->value = msm.pa; break;
	case MSM_PARAM_VA_SIZE:		p->value = POOL_SIZE; break;
	case MSM_PARAM_HIGHEST_BANK_BIT: p->value = 15; break;
	default:
		return (EINVAL);
	}
	return (0);
}

static int
gem_submit(struct mfile *mf, struct drm_msm_gem_submit *s)
{
	struct drm_msm_gem_submit_bo *bos = NULL;
	struct drm_msm_gem_submit_cmd *cmds = NULL;
	uint64_t *iova = NULL;
	uint32_t *ndw = NULL, i, n = 0;
	int error;

	if (s->nr_bos > 4096 || s->nr_cmds > 512 ||
	    (s->flags & (MSM_SUBMIT_FENCE_FD_IN | MSM_SUBMIT_SYNCOBJ_IN |
	    MSM_SUBMIT_SYNCOBJ_OUT | MSM_SUBMIT_FENCE_SN_IN)))
		return (EINVAL);
	bos = kmalloc(sizeof(*bos) * (s->nr_bos + 1), M_MSMDRM, M_WAITOK);
	cmds = kmalloc(sizeof(*cmds) * (s->nr_cmds + 1), M_MSMDRM, M_WAITOK);
	iova = kmalloc(sizeof(*iova) * (s->nr_cmds + 1), M_MSMDRM, M_WAITOK);
	ndw = kmalloc(sizeof(*ndw) * (s->nr_cmds + 1), M_MSMDRM, M_WAITOK);
	if ((error = copyin((void *)(uintptr_t)s->bos, bos,
	    sizeof(*bos) * s->nr_bos)) != 0 ||
	    (error = copyin((void *)(uintptr_t)s->cmds, cmds,
	    sizeof(*cmds) * s->nr_cmds)) != 0)
		goto out;
	for (i = 0; i < s->nr_bos; ++i) {
		struct gbo *bo = lookup(mf, bos[i].handle);

		if (bo == NULL) {
			error = EINVAL;
			goto out;
		}
		bos[i].presumed = bo->pa;
	}
	for (i = 0; i < s->nr_cmds; ++i) {
		struct gbo *bo;

		if (cmds[i].submit_idx >= s->nr_bos || cmds[i].nr_relocs) {
			error = EINVAL;
			goto out;
		}
		if (cmds[i].type == MSM_SUBMIT_CMD_IB_TARGET_BUF)
			continue;
		bo = lookup(mf, bos[cmds[i].submit_idx].handle);
		if ((uint64_t)cmds[i].submit_offset + cmds[i].size > bo->size) {
			error = EINVAL;
			goto out;
		}
		iova[n] = bo->pa + cmds[i].submit_offset;
		ndw[n] = cmds[i].size / 4;
		n++;
	}
	/* make everything the GPU will read visible, drop stale lines */
	for (i = 0; i < s->nr_bos; ++i) {
		struct gbo *bo = lookup(mf, bos[i].handle);

		cache_wbinv(bo->pa, bo->size);
	}
	s->fence = ++msm.seqno;
	error = a5xx_dfly_submit(iova, ndw, n, s->fence);
	if (error)
		goto out;
	for (i = 0; i < s->nr_bos; ++i)
		lookup(mf, bos[i].handle)->fence = s->fence;
	if (s->flags & MSM_SUBMIT_FENCE_FD_OUT)
		s->fence_fd = -1;	/* no fence fds yet */
out:
	kfree(bos, M_MSMDRM);
	kfree(cmds, M_MSMDRM);
	kfree(iova, M_MSMDRM);
	kfree(ndw, M_MSMDRM);
	return (error);
}

static int
msm_ioctl(struct dev_ioctl_args *ap)
{
	struct mfile *mf;
	void *data = ap->a_data;
	u_long cmd = ap->a_cmd;
	int nr = cmd & 0xff, error;

	if (IOCGROUP(cmd) != DRM_IOCTL_BASE)
		return (ENOTTY);
	if ((error = devfs_get_cdevpriv(ap->a_fp, (void **)&mf)) != 0)
		return (error);

	switch (nr) {
	case 0x00: {			/* DRM_IOCTL_VERSION */
		struct drm_version *v = data;

		v->version_major = MSM_VERSION_MAJOR;
		v->version_minor = MSM_VERSION_MINOR;
		v->version_patchlevel = MSM_VERSION_PATCH;
		error = copyout_str("msm", v->name, &v->name_len);
		if (error == 0)
			error = copyout_str("20260101", v->date, &v->date_len);
		if (error == 0)
			error = copyout_str("MSM Snapdragon DRM (DragonFly)",
			    v->desc, &v->desc_len);
		return (error);
	}
	case 0x09: {			/* DRM_IOCTL_GEM_CLOSE */
		struct drm_gem_close *c = data;
		struct gbo *bo;

		lockmgr(&msm.lk, LK_EXCLUSIVE);
		if ((bo = lookup(mf, c->handle)) != NULL) {
			mf->h[c->handle] = NULL;
			gbo_unref(bo);
		} else
			error = EINVAL;
		lockmgr(&msm.lk, LK_RELEASE);
		return (error);
	}
	case 0x0c: {			/* DRM_IOCTL_GET_CAP */
		struct drm_get_cap *c = data;

		c->value = 0;
		return (0);
	}
	case 0x0d:			/* DRM_IOCTL_SET_CLIENT_CAP */
		return (0);
	}
	if (nr < DRM_COMMAND_BASE)
		return (EINVAL);

	lockmgr(&msm.lk, LK_EXCLUSIVE);
	switch (nr - DRM_COMMAND_BASE) {
	case DRM_MSM_GET_PARAM:
		error = get_param(data);
		break;
	case DRM_MSM_SET_PARAM:
		break;
	case DRM_MSM_GEM_NEW: {
		struct drm_msm_gem_new *g = data;
		struct gbo *bo = gbo_alloc(g->size);

		if (bo == NULL)
			error = ENOMEM;
		else if ((error = new_handle(mf, bo, &g->handle)) != 0)
			gbo_unref(bo);
		break;
	}
	case DRM_MSM_GEM_INFO: {
		struct drm_msm_gem_info *gi = data;
		struct gbo *bo = lookup(mf, gi->handle);

		if (bo == NULL) {
			error = EINVAL;
			break;
		}
		switch (gi->info) {
		case MSM_INFO_GET_OFFSET:
		case MSM_INFO_GET_IOVA:
			gi->value = bo->pa;
			break;
		case MSM_INFO_SET_IOVA:
			error = (gi->value == bo->pa) ? 0 : EINVAL;
			break;
		case MSM_INFO_GET_FLAGS:
			gi->value = 0;
			break;
		case MSM_INFO_SET_NAME:
		case MSM_INFO_SET_METADATA:
			break;
		case MSM_INFO_GET_NAME:
		case MSM_INFO_GET_METADATA:
			gi->len = 0;
			break;
		default:
			error = EINVAL;
		}
		break;
	}
	case DRM_MSM_GEM_CPU_PREP: {
		struct drm_msm_gem_cpu_prep *p = data;
		struct gbo *bo = lookup(mf, p->handle);

		if (bo == NULL) {
			error = EINVAL;
			break;
		}
		if (p->op & MSM_PREP_NOSYNC)
			error = fence_done(bo->fence) ? 0 : EBUSY;
		else {
			uint32_t f = bo->fence;

			lockmgr(&msm.lk, LK_RELEASE);
			error = fence_wait(f, 5 * hz);
			lockmgr(&msm.lk, LK_EXCLUSIVE);
		}
		if (error == 0)
			cache_wbinv(bo->pa, bo->size);	/* see the GPU's writes */
		break;
	}
	case DRM_MSM_GEM_CPU_FINI: {
		struct drm_msm_gem_cpu_fini *p = data;
		struct gbo *bo = lookup(mf, p->handle);

		if (bo == NULL)
			error = EINVAL;
		else
			cache_wbinv(bo->pa, bo->size);	/* the cpu's writes out */
		break;
	}
	case DRM_MSM_GEM_SUBMIT:
		error = gem_submit(mf, data);
		break;
	case DRM_MSM_WAIT_FENCE: {
		struct drm_msm_wait_fence *w = data;
		uint32_t f = w->fence;

		lockmgr(&msm.lk, LK_RELEASE);
		error = fence_wait(f, (w->timeout.tv_sec || w->timeout.tv_nsec) ?
		    5 * hz : 0);
		lockmgr(&msm.lk, LK_EXCLUSIVE);
		break;
	}
	case DRM_MSM_GEM_MADVISE: {
		struct drm_msm_gem_madvise *m = data;

		if (lookup(mf, m->handle) == NULL)
			error = EINVAL;
		else
			m->retained = 1;
		break;
	}
	case DRM_MSM_SUBMITQUEUE_NEW: {
		struct drm_msm_submitqueue *q = data;

		q->id = mf->nextq++;
		break;
	}
	case DRM_MSM_SUBMITQUEUE_CLOSE:
		break;
	case DRM_MSM_SUBMITQUEUE_QUERY: {
		struct drm_msm_submitqueue_query *q = data;
		uint32_t zero = 0;

		if (q->param != MSM_SUBMITQUEUE_PARAM_FAULTS) {
			error = EINVAL;
			break;
		}
		if (q->data && q->len >= sizeof(zero))
			error = copyout(&zero, (void *)(uintptr_t)q->data,
			    sizeof(zero));
		q->len = sizeof(zero);
		break;
	}
	default:
		error = EINVAL;
	}
	lockmgr(&msm.lk, LK_RELEASE);
	return (error);
}

/* mmap offsets are physical addresses inside the pool */
static int
msm_mmap(struct dev_mmap_args *ap)
{
	vm_paddr_t off = ap->a_offset;

	if (off < msm.pa || off >= msm.pa + POOL_SIZE)
		return (EINVAL);
	ap->a_result = atop(off);
	return (0);
}

/* ---- set-up ---- */

static int
sysctl_ready(SYSCTL_HANDLER_ARGS)
{
	int v = msm.up;

	return (sysctl_handle_int(oidp, &v, 0, req));
}
SYSCTL_DECL(_hw_mi5);
SYSCTL_PROC(_hw_mi5, OID_AUTO, gpu_ready, CTLTYPE_INT | CTLFLAG_RD, 0, 0,
    sysctl_ready, "I", "1 when /dev/dri/card0 serves the GPU");

/* Called once the GPU runs (a5xx_dfly_start succeeded). */
void
msm_dfly_attach(void)
{
	if (msm.up || msm.kva == NULL)
		return;
	make_dev(&msm_ops, 0, UID_ROOT, GID_WHEEL, 0666, "dri/card0");
	make_dev(&msm_ops, 128, UID_ROOT, GID_WHEEL, 0666, "dri/renderD128");
	msm.up = 1;
	kprintf("msm: /dev/dri/card0: Adreno 530, %zu MB buffer pool at "
	    "0x%jx\n", POOL_SIZE >> 20, (uintmax_t)msm.pa);
}

/* The pool, early, while memory is still unfragmented. */
static void
msm_dfly_pool(void *dummy __unused)
{
	if (!msm8996_is_present())
		return;
	lockinit(&msm.lk, "msmdrm", 0, 0);
	msm.kva = contigmalloc(POOL_SIZE, M_MSMDRM, M_WAITOK, 0,
	    0xffffffffUL, 2 * 1024 * 1024, 0);
	if (msm.kva == NULL) {
		kprintf("msm: no %zu MB buffer pool\n", POOL_SIZE >> 20);
		return;
	}
	msm.pa = vtophys(msm.kva);
	msm.npages = POOL_SIZE / PAGE_SIZE;
	msm.used = kmalloc(msm.npages, M_MSMDRM, M_WAITOK | M_ZERO);
}
SYSINIT(msmdfly, SI_SUB_DRIVERS, SI_ORDER_ANY, msm_dfly_pool, NULL);
