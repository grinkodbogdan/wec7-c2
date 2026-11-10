# DragonFly BSD on arm64

An in-progress port of [DragonFly BSD](https://www.dragonflybsd.org/) to 64-bit
ARM (aarch64). DragonFly runs only on x86-64 upstream; this is the first part of
the work needed to change that.

**This is not a usable operating system yet.** The real DragonFly kernel boots
on an emulated ARM64 machine, mounts a root filesystem from a ramdisk and runs
a first user program, which can fork, take page faults and talk to the console.
What it cannot run is real software: there is no C library or toolchain for
this platform, so that one hand-written test program is all there is. It also
uses one CPU only and has no hardware drivers. See [What works](#what-works)
and [What is missing](#what-is-missing).

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
| `user/init.c` | The first user program: a freestanding `/sbin/init` that tests the kernel and offers a small prompt |
| `build.sh` | Fetch, overlay, build the kernel and ramdisk, and run them in QEMU |
| `tools/mi-coverage.sh` | Compiles only the shared kernel sources for aarch64 and reports failures |

The naming follows DragonFly's existing split: `cpu/x86_64` + `platform/pc64`
becomes `cpu/aarch64` + `platform/arm64`.

## Building and running

Requirements: `git`, `clang`, `ld.lld`, `llvm-objcopy`, `llvm-nm`,
`genisoimage`, and `qemu-system-aarch64` to run. No cross-GCC is needed.

```sh
./build.sh build     # fetch DragonFly (~900MB checkout), build kernel + ramdisk
./build.sh run       # boot in QEMU on your terminal; Ctrl-A X exits
```

`build.sh` stands in for `config(8)` and the kernel makefiles, which do not
know about arm64 yet. Expected console output (abridged):

```
Copyright (c) 2003-2026 The DragonFly Project.
...
DragonFly/arm64 94448f62 #0: Tue Nov 10 16:20:32 UTC 2026
Machine: linux,dummy-virt
CPU: ARM Cortex-A72 r0p3 (midr 0x410fd083)
real memory  = 1073741824 (1024 MB)
gic0: GICv2, distributor 0x8000000, cpu interface 0x8010000, 192 irqs
avail memory = 1027399680 (979 MB)
...
md0: Preloaded image <initrd> 434176 bytes at 0xfffff80048000000
Mounting root from cd9660:md0
cd9660: RockRidge Extension
Mounting devfs

Hello from user mode on DragonFly/aarch64 (pid 1)

  ok      getpid() == 1
  ok      write to .data
  ok      mmap anonymous memory
  ok      demand-zero page reads 0
  ok      page fault on write
  ok      fork, child exit status 7
  ok      copy-on-write after fork
pid 3 (init): data abort (user) at pc 0x400d4c, addr 0x8, signal 11
pid 3 (init), uid 0: exited on signal 11
  ok      child killed by SIGSEGV

Commands: test, uptime, reboot, poweroff
init>
```

The SIGSEGV line is part of the test: a child process dereferences a bad
pointer on purpose and the parent checks that it was killed. `poweroff` shuts
the kernel down cleanly and exits QEMU.

Boot arguments work as on other platforms: `QEMU_APPEND="-v"` for verbose
boot, `-s` for single user, `name=value` for loader tunables, and `selftest`
to run the atomic-primitive checks. Other knobs: `QEMU_CPU=max`,
`QEMU_MEM=3G`, `QEMU_EXTRA="-M virt,virtualization=on"` (enter at EL2).
Without a ramdisk (`vfs.root.mountfrom` unset) the kernel stops at the
`mountroot>` prompt.

## What works

Tested under QEMU 8.2 `virt` with Cortex-A53, Cortex-A72 and `max` CPU models,
256MB to 3GB of RAM, entered at both EL1 and EL2. One CPU only. The user
program's checks pass on all of them.

- **The kernel links and runs.** All 264 source files that `sys/conf/files`
  marks `standard` are compiled and linked with the arm64 platform code into a
  2.8MB kernel. Six shared files needed a one-line `__aarch64__` addition.
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
- **Root filesystem.** A ramdisk handed over the Linux way (QEMU `-initrd`, or
  the ramdisk slot of an Android boot image) becomes memory disk `md0` and is
  mounted as an ISO 9660 root.
- **User mode.** Programs are loaded by the standard ELF loader and run at
  EL0: system calls, page faults, fork with copy-on-write, exit status, and
  processes killed by a signal. `/dev/console` can be read and written.
- **Reboot and power-off** through PSCI, including a clean shutdown requested
  from user mode.

## What is missing

Roughly in the order it blocks progress:

| Area | State | What it takes |
|---|---|---|
| Software to run | One hand-written test program | A cross toolchain for `aarch64-dragonfly`, then libc, the runtime linker and the thread library, all of which have per-CPU assembly. This is now the largest gap |
| User mode, remaining | Basics work | Signal handlers (`sendsig`/`sigreturn`), FP/SIMD state (any FP instruction in user mode currently kills the process), thread-local storage system calls, debugger support |
| Terminal | Raw read/write only | A real tty driver for the serial port, needed before a shell or login can run |
| Storage | Ramdisk only, read-only ISO 9660 | A device-tree bus so drivers can attach, bus DMA (`stubs.c` fails every request), then virtio-blk and a writable filesystem |
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
2. ~~A root filesystem and a first user process.~~ Done, with a ramdisk and
   a test program.
3. **Real userland.** Signal handlers, FP/SIMD, TLS, a tty driver, and a
   cross-built libc, `/sbin/init` and shell. This is the largest step.
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
