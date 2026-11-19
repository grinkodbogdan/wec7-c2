# Decisions

Positions taken on the specification's open engineering decisions (§12).
Each is a working decision for the prototype, to be revisited with evidence.

| # | Question (spec §12) | Decision for now | Why / what would change it |
| --- | --- | --- | --- |
| D1 | Kernel language and first architecture | **Rust**, `no_std`; **arm64** first, on **QEMU `virt`** before real hardware | Memory safety (spec goal); arm64 is the phone architecture. QEMU gives a reproducible M1–M3 target. |
| D2 | Framekernel boundary | Kernel keeps translation, frames, scheduling, IRQ routing, IPC, capability checks, image loading. **Drivers are never in the kernel**; they get MMIO and IRQ capabilities. | Smallest set that still lets an unprivileged driver work. Image loading and composition move to the user-space component manager in M2. |
| D3 | IPC model | Synchronous rendezvous over endpoints, **register-only messages** (badge, label, 6 words), plus **notification bits** on endpoints for interrupts and events | Fast and simple; covers control traffic. Bulk data will use shared-memory capabilities (M3) rather than bigger messages. |
| D4 | Capability representation, delegation, revocation | Per-component slot table (32 slots) over a single **capability derivation tree**; rights only shrink on derive; one-time badging; revoke deletes the whole derived subtree; component death revokes everything it held | seL4-style provenance makes "take authority back" (hot-swap, faults) a single operation. Persistence is not addressed yet. |
| D5 | Driver ABI stability and versioning | Services are **versioned interfaces** (`name@MAJOR.MINOR`); minor versions only add; resolution and swap planning enforce compatibility | Lets drivers be replaced independently of their clients. |
| D6 | FHD syntax and relation to device tree | Own small, strict, **layered (SoC/board)** TOML-like syntax, validated at build time; DT-like vocabulary (compatible, reg, interrupts) so DTS can be imported later | Readable diffs for ports, precise errors, no silent defaults. |
| D7 | Graphics stack and buffer sharing | **Shared-memory capabilities plus standard KMS for Linux.** A display driver owns the panel and scans out a named shared-memory region (`shm:` grant) whose header carries the mode (`flare.display@1.0`). The Linux personality offers it as `/dev/dri/card0`, so Mesa and compositors (Arcan) run unmodified. Rendering is in software for now | Keeps the panel driver small and restartable (restart = hotplug), and reuses the whole Linux graphics stack instead of writing a new one. GPU drivers would export buffers into the same shared-memory objects. |
| D8 | Linux ABI placement | **In the framekernel for now** (`kernel/src/linux`, Asterinas-style), isolated in its own module with no Flare capabilities granted to Linux processes; console I/O goes through the user-space console driver | Fastest route to running real binaries. Moving it to a privileged personality component (split design) stays on the table once the ABI surface is known. |
| D9 | Android strategy | **Apps without Android userspace**: `droid` loads APKs and runs their DEX bytecode on its own interpreter, with the Java core and Android API implemented natively over Flare services. No init, servicemanager, zygote, ART or system_server. The Linux ABI, dynamic linking and Binder stay available for native code, and AOSP components remain an explicit fallback | Smallest path to running apps. Startup is milliseconds and memory is what the app uses. Missing APIs show up precisely, one at a time ([android.md](android.md)). |
| D10 | Boot chain, verified boot, updates | Open | |

## Implementation choices worth knowing

* **Single CPU, interrupts masked in the kernel, kernel never blocks.**
  Removes locking from M1 entirely. SMP needs per-CPU scheduler state and
  locks around shared objects.
* **1 GiB identity blocks for the kernel.** The build rejects boards whose
  device windows share a gigabyte with RAM or sit above `0x2_0000_0000`;
  lifting this means switching the kernel to a higher-half mapping with
  finer-grained device mappings.
* **The kernel log has a single writer.** Once a user-space console driver
  binds it, the kernel stops touching the UART.
* **Test policy is explicit.** `halt-when-finished` in a composition powers
  the system off once named components finish, so the QEMU test is
  deterministic.
