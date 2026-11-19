# Roadmap

Milestones from the project specification (§11), with status.

| Milestone | Status | Notes |
| --- | --- | --- |
| **M0 — Architecture contracts** | **Done (v0)** | Kernel boundary and ABI ([abi.md](abi.md)), capability model, component manifest, boot composition, versioned interfaces, lifecycle, hot-swap planning ([components.md](components.md)), FHD v0 ([fhd.md](fhd.md)), decisions ([decisions.md](decisions.md)) |
| **M1 — Framekernel prototype** | **Done on QEMU arm64** | Boots on QEMU `virt`; MMU and per-component address spaces with ASIDs; frame allocator; GICv2 and timer from FHD; preemptive round-robin scheduler; synchronous IPC with notifications; capability derivation and revocation; fault containment with restart policy; user-space UART driver. Verified by `cargo xtask test`. |
| M2 — Component manager | Next | See below |
| M3 — First hardware services | | Serial (done early), storage (virtio-blk), display, input |
| M4 — First phone port | | One well-understood phone/SoC |
| M5 — Linux ABI baseline | | |
| M6 — Android baseline | | |
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
