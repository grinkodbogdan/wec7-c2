# Roadmap

Milestones from the project specification (§11), with status.

| Milestone | Status | Notes |
| --- | --- | --- |
| **M0 — Architecture contracts** | **Done (v0)** | Kernel boundary and ABI ([abi.md](abi.md)), capability model, component manifest, boot composition, versioned interfaces, lifecycle, hot-swap planning ([components.md](components.md)), FHD v0 ([fhd.md](fhd.md)), decisions ([decisions.md](decisions.md)) |
| **M1 — Framekernel prototype** | **Done on QEMU arm64** | Boots on QEMU `virt`; MMU and per-component address spaces with ASIDs; frame allocator; GICv2 and timer from FHD; preemptive round-robin scheduler; synchronous IPC with notifications; capability derivation and revocation; fault containment with restart policy; user-space UART driver. Verified by `cargo xtask test`. |
| M2 — Component manager | **Partly** | Run-time supervisor: stop/start/restart any component, capabilities revoked and re-granted, clients reconnect (`/dev/flarectl`). Still in the kernel; moving it to user space is open |
| M3 — First hardware services | **Done on QEMU** | User-space drivers for every device on the reference board: UART console, PL031 RTC, PL061 power key, virtio-blk storage with DMA, ramfb display (shared memory, KMS for Linux) and virtio-input touch and keys (evdev). The Arcan phone shell runs on them ([display.md](display.md)) |
| M4 — First phone port | **Booting on hardware** | Xiaomi Mi A1 (MSM8953): SoC and board FHD, a boot image for its stock boot loader (the kernel moves itself to its link address), the console on the panel ([ports/xiaomi-tissot.md](ports/xiaomi-tissot.md)). Native drivers for the easy parts (volume key and hall sensor, debug UART, the panel through MDP5); Linux's own drivers in the Linux driver host for the hard ones ([linux-drivers.md](linux-drivers.md)). On the phone: Linux's eMMC, touch screen, keys, RPM, clocks and PMICs work. The full image hands touch and keys to Flare and starts the Arcan phone shell from a ramdisk root filesystem. Checked by `cargo xtask linuxdrv-test` (that image's shape on QEMU, unlocked by a swipe), `fbcon-test` and `tools/bootimg/early-boot.sh`. Next: the shell on the phone itself, eMMC storage for Flare, USB (fdb) |
| **M5 — Linux ABI baseline** | **Running** | busybox, Rust `std`, and QRT's Linux conformance suite (threads, signals, memory, events/Unix sockets, processes) pass; checked by `cargo xtask test`. Gaps: dynamic linking, internet sockets ([linux.md](linux.md)) |
| M6 — Android baseline | **Apps running, with UI** | Unmodified APKs (Arity from F-Droid) run on ART with Android Translation Layer's framework and draw natively into Arcan windows in the phone shell, with touch and keys ([android.md](android.md)); simple Java and Kotlin apps also run under `droid`, Flare's own DEX interpreter; dynamic linking and Binder are in place. Next: apps' native libraries, GL, dialogs, dex2oat |
| M7 — Portability validation | | Second, materially different phone |

## Next steps (M2)

1. **User-space component manager.** Kernel starts only the manager with
   all root capabilities; the manager runs `flare-cm` to apply the boot
   composition. Needs kernel objects for untyped memory/frames, address
   space and thread creation, and capability transfer over IPC (`grant`
   right).
2. **Runtime hot-swap.** Execute `flare_cm::swap` plans against live
   components: quiesce at the IPC boundary, revoke/transfer capabilities,
   reconnect clients. Demonstrate replacing the UART driver while clients
   keep running.
3. **Health monitoring.** Fault and exit reports delivered to the manager
   instead of handled in the kernel.
4. **Shared-memory capabilities** for bulk data (prerequisite for storage,
   display and the Linux personality).
5. **Multiple threads per component**, then SMP.
6. **Second board description** (a different QEMU machine or a real dev
   board) to exercise the FHD layering early.

## Known limitations

* One CPU; one thread per component; 32 capability slots per component.
* The kernel applies the boot composition itself (M1 stand-in for M2).
* No capability transfer over IPC yet; delegation is within a component
  (derive) and from the composition.
* Kernel memory map uses 1 GiB blocks (see [decisions.md](decisions.md)).
* Only GICv2, the arm generic timer and PSCI are supported platform
  primitives.
