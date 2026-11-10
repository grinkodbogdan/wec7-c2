# DragonFly BSD on arm64

An in-progress port of [DragonFly BSD](https://www.dragonflybsd.org/) to 64-bit
ARM (aarch64). DragonFly runs only on x86-64 upstream; this is the first part of
the work needed to change that.

**This is not a usable operating system yet.** The real DragonFly kernel now
boots on an emulated ARM64 machine, runs its whole startup sequence on one CPU
and stops at the prompt asking for a root filesystem. It cannot go further:
there is no way to run user programs yet, no second CPU, and no drivers. See
[What works](#what-works) and [What is missing](#what-is-missing).

## Will this run on the TicWatch C2?

No. The TicWatch C2 (Snapdragon Wear 2100, Cortex-A7) is a **32-bit** ARM
device. DragonFly dropped 32-bit support in 2014 and its kernel assumes 64-bit
pointers and `long` throughout, so the only realistic ARM target is 64-bit. A
32-bit port would be a second, larger project on top of this one.

Realistic first hardware for this port is anything with a GICv2 interrupt
controller and a PL011 serial port that boots Linux-style kernel images: QEMU's
`virt` machine today, a Raspberry Pi 4 with modest work. Phones come much later
(see [Roadmap](#roadmap)).

## Layout

DragonFly's source is not copied here. `build.sh` fetches it at a pinned commit
and lays this directory's `sys/` tree over it.

| Path | Contents |
|---|---|
| `sys/cpu/aarch64/include/` | CPU headers: atomic operations, system registers, trap frame, page table entry bits, signal/ELF/TLS definitions |
| `sys/platform/arm64/aarch64/` | The platform code: boot, exceptions, page tables (`pmap.c`), thread switching (`swtch.S`), interrupts, clock, console |
| `sys/platform/arm64/aarch64/stubs.c` | Every kernel entry point that is not implemented yet, in one place |
| `sys/platform/arm64/include/` | Platform headers: per-CPU data, thread, PCB, pmap, address space layout |
| `sys/platform/arm64/conf/` | Kernel linker script |
| `patches/mi-aarch64.diff` | Edits to six shared kernel files that tested for `__x86_64__` |
| `build.sh` | Fetch, overlay, build the kernel, and run it in QEMU |
| `tools/mi-coverage.sh` | Compiles only the shared kernel sources for aarch64 and reports failures |

The naming follows DragonFly's existing split: `cpu/x86_64` + `platform/pc64`
becomes `cpu/aarch64` + `platform/arm64`.

## Building and running

Requirements: `git`, `clang`, `ld.lld`, `llvm-objcopy`, `llvm-nm`, and
`qemu-system-aarch64` to run. No cross-GCC is needed.

```sh
./build.sh build     # fetch DragonFly (~900MB checkout), overlay, build
./build.sh run       # boot in QEMU on your terminal; Ctrl-A X exits
```

`build.sh` stands in for `config(8)` and the kernel makefiles, which do not
know about arm64 yet. Expected console output:

```
Copyright (c) 2003-2026 The DragonFly Project.
Copyright (c) 1992-2003 The FreeBSD Project.
Copyright (c) 1979, 1980, 1983, 1986, 1988, 1989, 1991, 1992, 1993, 1994
	The Regents of the University of California. All rights reserved.
DragonFly/arm64 94448f62 #0: Tue Nov 10 16:20:32 UTC 2026
Machine: linux,dummy-virt
CPU: ARM Cortex-A72 r0p3 (midr 0x410fd083)
  Physical address bits: 44, ASID bits: 16, running at EL1
  Features: FP AdvSIMD AES SHA2 CRC32
real memory  = 1073741824 (1024 MB)
gic0: GICv2, distributor 0x8000000, cpu interface 0x8010000, 192 irqs
avail memory = 1027870720 (980 MB)
CPU Topology: cores_per_chip: 1; threads_per_core: 1; chips_per_package: 1;
Initialize MI interrupts for 1 cpus
link_elf_lookup_symbol: missing symbol hash table
interrupt uses mplock: swi_taskq
wdog: In-kernel automatic watchdog reset enabled

Manual root filesystem specification:
  <fstype>:<device>  Specify root (e.g. ufs:da0s1a)
  ?                  List valid disk boot devices
  panic              Just panic
  abort              Abort manual input

mountroot>
```

The prompt takes keyboard input. `?` lists no devices because there are no
storage drivers; any answer ends in "Root mount failed", after which the kernel
reboots itself.

Boot arguments work as on other platforms: `QEMU_APPEND="-v"` for verbose
boot, `-s` for single user, `name=value` for loader tunables, and `selftest`
to run the atomic-primitive checks. Other knobs: `QEMU_CPU=max`,
`QEMU_MEM=3G`, `QEMU_EXTRA="-M virt,virtualization=on"` (enter at EL2).

## What works

Tested under QEMU 8.2 `virt` with Cortex-A53, Cortex-A72 and `max` CPU models,
256MB to 3GB of RAM, entered at both EL1 and EL2. One CPU only.

- **The kernel links and runs.** All 264 source files that `sys/conf/files`
  marks `standard` are compiled and linked with the arm64 platform code into a
  2.7MB kernel. Six shared files needed a one-line `__aarch64__` addition.
- **Boot.** The image carries a Linux arm64 `Image` header, so QEMU, U-Boot and
  Android bootloaders can load it. Memory size, serial port, interrupt
  controller, timer and boot arguments come from the device tree.
- **Virtual memory.** A complete, if simple, page-table manager (`pmap.c`):
  kernel memory grows on demand, pageable kernel memory faults in, and the
  code paths for user address spaces exist but have never run.
- **Threads and scheduling.** Kernel threads are created, switched and
  preempted by interrupt threads; the idle loop sleeps the CPU.
- **Interrupts and time.** GICv2 dispatch that honours critical sections, soft
  interrupts, and the ARM generic timer as both the system timebase and the
  scheduler's interrupt source. Uptime advances and timeouts fire.
- **Console.** DragonFly's own `kprintf` and console layer on a PL011 serial
  port, output and polled input.
- **Reboot and power-off** through PSCI.

## What is missing

Roughly in the order it blocks progress:

| Area | State | What it takes |
|---|---|---|
| User mode | Not started. Exceptions from user mode are fatal and there is no return path to it | System call entry, user page faults, signal delivery, `exec` register setup, FP/SIMD state switching. On x86-64 this is most of `trap.c`, `machdep.c` and `npx.c`, about 5,000 lines |
| Something to run | Nothing | A cross toolchain for `aarch64-dragonfly`, then libc, the runtime linker and the thread library, all of which have per-CPU assembly |
| Storage | No drivers | A device-tree bus so drivers can attach, bus DMA (`stubs.c` fails every request), then at least virtio-blk or a RAM disk |
| More than one CPU | Not started | Starting secondary CPUs through PSCI, inter-processor interrupts, and an audit of shared code that assumes x86's strong memory ordering |
| Interrupt controllers | GICv2 only | GICv3, which most hardware from the last several years uses, and MSI |
| pmap performance | Correct but slow | One global lock, no address-space IDs (every process switch flushes the TLB), no large pages, dirty/accessed tracking by over-approximation |
| Kernel modules | Cannot load | Relocation processing and a kernel symbol table |
| Debugging | None | The in-kernel debugger backend, crash dumps, `ptrace` register access |
| Build system | Bypassed | Teaching `config(8)` and the kernel makefiles about arm64 |
| Boot loader | QEMU `-kernel` only | A UEFI or U-Boot loader that can read HAMMER2 and pass modules |
| Real hardware | Untested | Memory reservations from the device tree are ignored, the UART is assumed to be a PL011 below 1GB, and nothing has run outside QEMU |

`sys/platform/arm64/aarch64/stubs.c` is the precise list of missing kernel
entry points: each one either fails with an error or panics with a message
naming what is missing, so hitting one is obvious.

### Known open questions

- **Thread-local storage layout.** DragonFly uses the x86 "variant II" TLS
  layout everywhere; the aarch64 ELF ABI specifies variant I. `cpu/tls.h`
  currently keeps variant II and notes the conflict. This must be settled
  before any userland is built.
- **Memory ordering.** x86 is strongly ordered and aarch64 is not. The atomic
  header uses the strongest ordering for every operation, which is safe, but
  shared kernel code that relies on plain loads and stores being ordered (it
  exists) will need auditing once SMP runs.
- **`char` signedness.** `char` is unsigned on aarch64. Kernel code that
  assumes signed `char` compiles fine and misbehaves at run time.

## Roadmap

1. ~~Reach `mi_startup()` on one CPU and boot to the root prompt.~~ Done.
2. **A root filesystem.** Device-tree bus, bus DMA, and a virtio-blk driver or
   memory disk so the kernel can mount something.
3. **User mode.** Exception entry and return for EL0, system calls, signals,
   and a cross-built `/sbin/init` and shell. This is the largest step.
4. **SMP.** Secondary CPUs, IPIs, and the memory-ordering audit.
5. **Real hardware.** GICv3 and drivers for one board (Raspberry Pi 4 or a
   well-documented Rockchip board).
6. **Self-hosting.** Native toolchain, `buildworld` on arm64.
7. **Phones.** Each phone needs its own serial, storage, display, touch, power
   and modem drivers, and vendors publish these only for Linux. Expect this to
   be a per-device project comparable in size to everything above.

For scale: FreeBSD's arm64 port was a funded, multi-year effort before it first
shipped in FreeBSD 11.0, and DragonFly's kernel differs enough from FreeBSD's
(LWKT threading, its own pmap and VM) that little of that code transfers
directly.

## Design notes

- **`mycpu`** reads `TPIDR_EL1`, which holds the per-CPU `globaldata` pointer,
  in place of the `%gs` segment on x86-64.
- **Page tables** use a 4K granule and four levels, which has the same shape as
  x86-64's. `cpu/param.h` keeps the x86 level names (`PML4`, `PDP`, `PDR`) as
  aliases so the eventual pmap can stay structurally close to pc64's. Unlike
  x86, user and kernel halves have separate roots (`TTBR0`/`TTBR1`).
- **`rdtsc()`** returns the generic timer's virtual counter. Shared code calls
  it by name for lock-contention timing and entropy; the counter has the
  properties those callers want.
- **Names borrowed from x86** (`read_rflags`, `get_cpuid_from_apicid`,
  `PAT_*`, `machine/specialreg.h`) are provided where shared code uses them
  unconditionally. Each has a comment at its definition saying what it maps to.
- **Thread switching** follows the protocol `kern/lwkt_thread.c` expects and
  pc64's `swtch.s` implements, with x0/x19 in the roles of `%rax`/`%rbx`. The
  comment at the top of `swtch.S` describes the stack frame.
- **TLB invalidation** uses the broadcast forms of `TLBI`, so the IPI-based
  shootdown machinery pc64 needs (`pmap_inval.c`) has no counterpart.
- **Dirty-page tracking** is by invariant instead of hardware bits: a page with
  any writable mapping is treated as dirty. The top of `pmap.c` explains why
  this is safe and what it costs.
- **Known shortcuts:** the single-pass device tree scanner, the byte-loop
  `memcpy`/`memset`/`copyin` in `support.S`, and the console's polled input.
