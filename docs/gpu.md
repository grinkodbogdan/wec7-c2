# GPU: Linux's driver, shared efficiently

**Status: design, not built yet.** The phone shell renders in software
(Mesa softpipe) today.

The Xiaomi Mi A1's GPU (Adreno 506) has a mature driver in Linux: `msm`
(DRM), with Mesa's `freedreno` in user space. Flare should use both
unchanged rather than port either. The pieces are already in place:
* Linux's drivers run in the driver host ([linux-drivers.md](linux-drivers.md));
* Flare programs use Mesa through the Linux personality ([display.md](display.md)).

What is missing is the path between them.

## The idea: a DRM render node proxied to the driver host

```
Flare program (Arcan, Mesa freedreno)
   │  ioctl(/dev/dri/renderD128, DRM_IOCTL_MSM_*)
Flare kernel (Linux personality)
   │  queue the request; block the caller          ← small structs only
driver host (linuxdrv)
   │  the same ioctl on Linux's /dev/dri/renderD128 (inside LKL)
Linux's msm driver → the GPU
```

1. **Commands go by ioctl forwarding.** Freedreno talks to the kernel through a
   few dozen small ioctls (GEM_NEW, GEM_INFO, GEM_CPU_PREP, GEM_SUBMIT,
   WAIT_FENCE, SUBMITQUEUE_*). The Flare kernel serves
   `/dev/dri/renderD128` by queueing each ioctl for the driver host (a new
   `/dev/flarehw` request queue, like FUSE). The host issues it on Linux's
   real render node and returns the result. Pointers inside the
   structures (the submit's buffer and command tables) are copied by the
   kernel both ways. They are bounded and small.

2. **Buffers are not copied: they are mapped.** Linux's RAM in the driver
   host is one physically contiguous block that the Flare kernel handed
   out (`FLAREHW_DMA_ALLOC`, LKL's MMU mode). Every GEM buffer Linux's
   driver allocates lives in it, at physical addresses the Flare kernel can
   check. When a Flare program mmaps a buffer, the host asks Linux for its
   pages, and the Flare kernel maps those physical pages into the program,
   write-combined for GPU buffers. Vertex data, textures and render targets
   are written once, in place, as on Linux.

3. **Frames go to the panel without copies.** A finished frame is a GEM
   buffer in that same RAM. Its physical address can be given to the
   display driver (`drivers/qcom/mdp5-display`), which points the MDP5
   pipe at it at the next refresh (QRT's page flip, 0.28.3). That replaces
   today's CPU copy of every frame into the display memory. Arcan's KMS
   path stays as it is, with dma-buf import (PRIME) between the render node
   and `/dev/dri/card0`.

4. **Interrupts and power are Linux's.** The GPU's interrupt, clocks,
   GDSC and RPM votes are Linux's, through the same `/dev/flarehw` grants
   as the eMMC (`[linux] hardware = ["gpu", "gpu-iommu", ...]`).

## What Linux's driver needs on this phone (from QRT's bring-up)

* **The GPU's IOMMU** (`qcom,msm-iommu-v2` at 0x1c48000). Its secure
  context needs SCM calls (secure page table size and setup): those
  function ids go into the composition's `[linux] smc` allow-list, and no
  others (QRT 0.21.15, 0.26.2).
* **Firmware**: `a530_pm4.fw` and `a530_pfp.fw` (linux-firmware), plus the
  zap shader `a506_zap.mdt`/`.b0x` from the phone's firmware partition. It
  is loaded through the secure world (PAS SCM calls, allowed likewise).
  The driver host gives Linux a `/lib/firmware` from the root filesystem
  (QRT 0.26.3).
* `CONFIG_DRM_MSM` with the display parts off (the panel stays Flare's),
  `CONFIG_QCOM_IOMMU`, `CONFIG_QCOM_SCM` with the SCM node given back to
  Linux for these calls only.
* Mesa built with `-Dgallium-drivers=freedreno,kmsro,softpipe`. Arcan's
  EGL then picks freedreno for rendering and Flare's KMS for display.

## Cost per frame, compared

| | today (softpipe) | proxied msm |
| --- | --- | --- |
| rendering | all on the CPU (A53s) | on the GPU |
| per frame across the boundary | 8 MB copy + cache clean | a few ioctls (~1–2 KB) |
| scanout | CPU copy into the display memory | the MDP5 pipe re-pointed at the frame |

## Order of work

1. The request queue in `/dev/flarehw`, and `/dev/dri/renderD128`
   forwarding, tested in QEMU with Linux's `virtio-gpu` driver in the
   driver host (the same DRM ioctl path, `virgl` off: dumb GEM only).
2. GEM mmap by physical pages from the driver host's RAM (QEMU as above).
3. On the phone: the GPU, its IOMMU, firmware and the SMC allow-list;
   `fd_rd`-style test submits from a Flare program.
4. Mesa freedreno, then Arcan on it; then MDP5 page flips to GPU frames.
