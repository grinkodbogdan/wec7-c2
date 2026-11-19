# Flare OS

A modular, capability-secure operating system for phones, with hot-pluggable
hardware components and multiple application compatibility environments.

Flare combines a small privileged **framekernel** (Rust, arm64) with drivers
and services that run as **isolated user-space components**. Components get
**no ambient authority**: everything they can touch — device registers,
interrupts, IPC endpoints, even the console — is an explicit capability
granted by a boot composition. Hardware is described in **FHD** files, so a
phone port is hardware descriptions, driver components and configuration —
not kernel patches.

Status: **M0 (architecture contracts) done, M1 (framekernel prototype)
booting on QEMU arm64.** See [docs/roadmap.md](docs/roadmap.md).

## Quick start

Requirements: a Rust toolchain (`rustup` installs the pinned target from
`rust-toolchain.toml`) and `qemu-system-aarch64`.

```sh
cargo xtask test        # host unit tests + boot the system in QEMU and check its behaviour
cargo xtask run         # boot interactively (type a line when asked; Ctrl-C quits)
cargo xtask fhd qemu/virt-phone      # validate and show a board's hardware description
cargo xtask cm resolve               # interface wiring and start order of the boot composition
cargo xtask cm swap uart drivers/common/uart-pl011   # plan a hot-swap of a component
```

What the reference system demonstrates when it boots:

```
kernel: started 'uart' (incarnation 0, asid 1, 14 pages, 4 capabilities)
uart-pl011: console service online (incarnation 0, user-space driver)
hello: Hello from a native Flare component 'hello' (incarnation 0)
hello: this line travelled through a derived capability
hello: derived capability revoked -> E_NO_CAP as expected
spinner: starting a CPU-bound loop that never yields
intruder: derive RECV from SEND-only console cap -> denied (E_RIGHTS)
intruder: reading kernel memory at 0x40080000
kernel: component 'intruder' faulted: data abort (permission fault) at 0x40080000, pc 0x20000031c; stopping it
kernel: restarting 'intruder' (1/1)
...
kernel: component 'intruder' retired
flare
hello: received input: flare
flare: system halted after 197 ticks (9 capabilities live, 38 frames in use)
```

* The **UART driver is an unprivileged component**: it owns the PL011
  registers (MMIO capability), receives its interrupt as an IPC notification
  (IRQ capability) and serves `flare.console@1.0` to the other components.
  The kernel log is routed through it too.
* **Capabilities** are checked on every system call; derived capabilities
  can only lose rights and can be revoked through their parent.
* A **misbehaving component** is contained: it is stopped on a fault,
  restarted according to its policy, then retired. Nothing else is affected.
* A component that **never yields** does not stall the system (timer
  preemption).

## Repository layout

Follows the illustrative layout of the project specification (§7):

| Path | What |
| --- | --- |
| `kernel/` | The framekernel: boot, MMU, GICv2, timer, frames, address spaces, capabilities, IPC, scheduling, syscalls |
| `interfaces/abi/` | `flare-abi`: the kernel ABI and service protocols — the contract between kernel and components |
| `component-manager/` | `flare-cm`: manifests, interface resolution, lifecycle, hot-swap planning, driver binding, boot compositions |
| `libs/cap/` | `flare-cap`: capability derivation tree with revocation |
| `libs/config/` | `flare-config`: the small TOML-like syntax shared by FHD files, manifests and compositions |
| `libs/rt/` | `flare-rt`: runtime for native components (entry, syscalls, console client) |
| `tools/fhd/` | `flare-fhd`: Flare Hardware Description loader, SoC/board layering, validation, code generation |
| `hardware/soc/…/platform.fhd` | SoC-level hardware descriptions |
| `hardware/boards/…/board.fhd` | Board (phone) descriptions layered over a SoC |
| `drivers/common/` | Driver components (today: `uart-pl011`) |
| `apps/` | Demo native components (`hello`, `spinner`, `intruder`) |
| `system/` | Boot compositions: which components start, with exactly which capabilities |
| `xtask/` | `cargo xtask` build/run/test driver |
| `docs/` | Architecture contracts and decisions |

## Documentation

* [Architecture](docs/architecture.md) — layers, kernel boundary, boot flow
* [Kernel ABI](docs/abi.md) — system calls, capabilities, IPC, boot info
* [Components](docs/components.md) — manifests, compositions, lifecycle, hot-swap
* [Flare Hardware Description](docs/fhd.md) — the FHD format and porting
* [Decisions](docs/decisions.md) — positions taken on the spec's open questions
* [Roadmap](docs/roadmap.md) — milestone status and next steps
