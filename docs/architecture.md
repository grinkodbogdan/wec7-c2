# Flare architecture

This document maps the layers of the project specification (§4) onto the
code, and records where the kernel boundary sits today.

## Layers

| Spec layer | Today | Code |
| --- | --- | --- |
| Applications and shell | Native demo components | `apps/`, `libs/rt` |
| Compatibility environments | Not started (M5, M6) | `compatibility/` (future) |
| Shared system services | `flare.console@1.0`, served by the UART driver | `interfaces/abi` (`proto`) |
| Component manager | Policy core as a library; the kernel applies the boot composition (M1 stand-in) | `component-manager/`, `kernel/src/kernel.rs` |
| Driver components | PL011 UART, unprivileged | `drivers/common/uart-pl011` |
| Framekernel | Address spaces, frames, scheduling, IRQ dispatch, IPC, capabilities | `kernel/` |
| Architecture / platform | arm64 boot, MMU, GICv2, generic timer, PSCI; hardware from FHD | `kernel/src/arch/aarch64`, `hardware/` |

## The kernel boundary (M1)

Privileged (in the framekernel):

* translation tables and ASIDs; the physical frame allocator
* exception entry, the scheduler (round robin, 100 Hz preemption)
* interrupt acknowledgement and routing to the owning component
* synchronous IPC and notifications
* the capability derivation tree and every authority check
* loading component images and the boot composition (moves to the
  user-space component manager in M2)
* an early console, used only until a log reader binds the kernel log

Everything else is a component. In particular **device drivers are not in
the kernel**: a driver receives its register window as an MMIO capability
(mapped into its address space as device memory) and its interrupt line as
an IRQ capability, which the kernel turns into an IPC notification.

The kernel contains **no board knowledge**. `kernel/build.rs` generates the
platform table (memory, interrupt controller, timer, devices, PSCI conduit)
and the link address from the board's FHD files, and the boot table from the
boot composition. Porting to a board with the same architecture and
interrupt-controller family is a description change.

## Concurrency model

One CPU. Interrupts are always masked in the kernel, and the kernel never
blocks: every entry (system call, fault, interrupt) runs to completion and
returns to some component, or idles in `wfi` until an interrupt arrives.
This removes all locking from the kernel. SMP is future work and will need
per-CPU state and locking around shared objects.

## Memory layout

* The kernel identity-maps RAM and device windows in 1 GiB blocks that only
  EL1 can access (`kernel/src/arch/aarch64/mmu.rs`).
* Each component address space copies those (inaccessible to EL0) entries
  and adds the user region at `0x2_0000_0000` with 4 KiB pages tagged with
  its ASID: image (W^X: code R-X, rodata R--, data RW-), MMIO window, boot
  info page (R--), stack. See [abi.md](abi.md#address-space-layout).

## Boot flow

1. `_start` (arm64): park secondary CPUs, drop from EL2 if needed, zero BSS,
   call `kmain`.
2. Turn on the MMU with the identity map generated from the FHD; set up the
   heap and early console; initialise frames, vectors, GIC and timer.
3. Create endpoints and spawn every boot component: load its ELF image, map
   its stack and boot-info page, derive each granted capability from the
   kernel's root capability and install it in the component's slots.
4. Enter user space. The UART driver binds its IRQ and the kernel log to its
   console endpoint and becomes the only writer to the UART.
5. Components that fault are torn down (all their capabilities revoked,
   memory freed, callers waiting on them woken with `E_PEER_DIED`) and
   restarted per their policy.

## Kernel log

Before a reader binds it, `kprintln!` writes to the UART directly. After a
component with `klog:read` binds it to an endpoint, kernel output is
buffered and the reader is notified; if the reader dies, the kernel flushes
the buffer and resumes direct output. This keeps a single writer on the
UART, so kernel and component output never interleave mid-line.
