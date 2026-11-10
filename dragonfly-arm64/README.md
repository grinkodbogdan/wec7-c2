# DragonFly BSD on arm64

An in-progress port of [DragonFly BSD](https://www.dragonflybsd.org/) to 64-bit
ARM (aarch64). DragonFly runs only on x86-64 upstream; this is the first part of
the work needed to change that.

**This is not a usable operating system yet.** What exists is the foundation of
the kernel port: it boots on an emulated ARM64 machine and proves the low-level
pieces work, and the whole machine-independent kernel compiles for ARM64, but
the two are not joined into a running DragonFly kernel. See
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
| `sys/platform/arm64/aarch64/` | Boot code, exception vectors, device tree scanner, early console, GICv2, timer |
| `sys/platform/arm64/include/` | Platform headers: per-CPU data, thread, PCB, pmap, address space layout |
| `sys/platform/arm64/conf/` | Kernel linker script |
| `patches/mi-aarch64.diff` | Edits to six shared kernel files that tested for `__x86_64__` |
| `build.sh` | Fetch, overlay, build, and run in QEMU |
| `tools/mi-coverage.sh` | Compiles the machine-independent kernel for aarch64 and reports failures |

The naming follows DragonFly's existing split: `cpu/x86_64` + `platform/pc64`
becomes `cpu/aarch64` + `platform/arm64`.

## Building and running

Requirements: `git`, `clang`, `ld.lld`, `llvm-objcopy`, and
`qemu-system-aarch64` to run. No cross-GCC is needed.

```sh
./build.sh build     # fetch DragonFly (~900MB checkout), overlay, build
./build.sh run       # boot in QEMU; the kernel powers off when done
tools/mi-coverage.sh # compile the machine-independent kernel for aarch64
```

Expected output of `./build.sh run`:

```
Copyright (c) 2003-2026 The DragonFly Project.
...
DragonFly/arm64 platform bring-up kernel (MI kernel not linked)

Machine: linux,dummy-virt (linux,dummy-virt)
FDT at 0x48000000 (1048576 bytes), bootargs ""
Console: PL011 at 0x9000000 (stdout-path /pl011@9000000)
CPU: ARM Cortex-A72 r0p3 (midr 0x410fd083)
  Physical address bits: 44, ASID bits: 16, running at EL1
  Features: FP AdvSIMD AES SHA2 CRC32
  4 cpu(s) in device tree, boot cpu only
Physical memory: 0x0000000040000000 - 0x000000007fffffff (1024 MB)
real memory = 1073741824 (1024 MB)
Kernel: physical 0x40200000, virtual 0xffffffff80000000 - 0xffffffff80011000 (text ends 0xffffffff80004000)
MMU: TTBR0 identity map retired, DMAP at 0xfffff80000000000

Self-test: atomic ops and bit primitives
Self-test: exception entry and fault recovery
trap: BRK #0 at 0xffffffff80001d48, resuming
GICv2: distributor 0x8000000, cpu interface 0x8010000, 288 irqs
Self-test: generic timer interrupt (irq 27)
  50 ticks at 100 Hz took 517 ms by the counter

All self-tests passed.
Next milestone: pmap bootstrap and mi_startup().  Powering off.
```

Useful variations: `QEMU_CPU=max`, `QEMU_MEM=3G`,
`QEMU_EXTRA="-M virt,virtualization=on"` (enter at EL2),
`QEMU_APPEND="..."` (kernel command line).

## What works

Tested under QEMU 8.2 `virt` with Cortex-A53, Cortex-A72 and `max` CPU models,
128MB to 3GB of RAM, entered at both EL1 and EL2.

- **Boot.** The kernel image carries a Linux arm64 `Image` header, so QEMU,
  U-Boot and Android bootloaders can load it. Entry code drops from EL2 to EL1
  if needed, builds page tables, and enables the MMU.
- **Address space.** The kernel runs at `0xffffffff80000000` with a direct map
  of physical memory at `0xfffff80000000000`, the same layout as x86-64
  DragonFly, so shared code's assumptions about it hold. The lower half is left
  empty for userland.
- **Device tree.** Memory size, CPU count, serial port, interrupt controller,
  timer interrupt and boot arguments are read from the device tree rather than
  hard-coded.
- **Exceptions.** Vectors save and restore a full trap frame. A breakpoint
  round-trips, and a deliberate read of unmapped memory is caught and recovered
  from.
- **Interrupts and timer.** GICv2 delivers the ARM generic timer interrupt at
  100Hz, cross-checked against the hardware counter.
- **Atomic and bit primitives.** Checked at boot against the return-value
  conventions the shared kernel relies on, including the interrupt-safe
  serializer lock.
- **Machine-independent kernel compiles.** All 264 source files that
  `sys/conf/files` marks `standard` (scheduler, VM, VFS core, networking core,
  the kernel's C library) compile to aarch64 objects against these headers.
  Six of them needed a one-line `__aarch64__` addition (`patches/`).

## What is missing

Compiling is not running. Linking those 264 objects leaves 232 unresolved
symbols (`tools/mi-coverage.sh` writes the list). About 160 are
machine-dependent functions that do not exist yet for arm64; the rest come from
optional or build-generated files. The big pieces, roughly in the order they
need to be written:

| Area | Missing functions (examples) | x86-64 equivalent | Size there |
|---|---|---|---|
| Virtual memory | `pmap_enter`, `pmap_remove`, `pmap_protect`, `pmap_growkernel`, ~60 more | `pmap.c`, `pmap_inval.c` | ~7,500 lines |
| Context switch | `cpu_heavy_switch`, `cpu_lwkt_switch`, `cpu_fork`, `cpu_set_thread_handler`, `savectx` | `swtch.s`, `vm_machdep.c` | ~2,200 lines |
| User/kernel copy | `copyin`, `copyout`, `copyinstr`, `fuword64`, `suword64` | `support.s` | ~900 lines |
| Traps and syscalls | real `trap()`, `syscall2`, `sendsig`, `sys_sigreturn`, `exec_setregs` | `trap.c`, `machdep.c` | ~5,000 lines |
| Interrupt framework | `MachIntrABI`, `splz`, soft interrupts, GICv3 | `apic/`, `icu/`, `ipl.s` | ~4,500 lines |
| SMP | `cpu_send_ipiq`, `smp_invltlb`, `stop_cpus`, secondary CPU start via PSCI | `mp_machdep.c` | ~2,000 lines |
| Clocks | `DELAY`, cputimer and cputimer-interrupt backends, `inittodr` | `isa/clock.c`, `apic/lapic.c` | ~3,000 lines |
| Bus and DMA | `bus_dmamap_load`, `bus_dmamem_alloc`, nexus, FDT bus | `busdma_machdep.c`, `nexus.c` | ~2,000 lines |
| Debug and dump | `fill_regs`, `ptrace_set_pc`, ddb backend, minidump | `db_*.c`, `*dump_machdep.c` | ~1,900 lines |
| Floating point | lazy save/restore of FP/SIMD state | `npx.c` | ~500 lines |

Beyond the kernel, a working system also needs:

- **Build system integration.** `config(8)`, `sys/conf` makefiles and
  `Makefile.inc1` know only x86-64. `build.sh` sidesteps them for now.
- **Toolchain.** DragonFly's in-tree GCC and binutils need an
  `aarch64-dragonfly` target.
- **Userland.** libc (system call stubs, `setjmp`, string routines), the
  runtime linker, thread library and C startup files all have per-CPU assembly.
- **Boot loader.** A UEFI or U-Boot loader that can read HAMMER2 and hand the
  kernel its modules.
- **Drivers.** Nothing beyond the early serial port exists. Storage, network,
  USB and display are all per-platform work, and existing DragonFly drivers
  assume PCI and ACPI.

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

1. **Reach `mi_startup()` on one CPU.** Write the pmap bootstrap, context
   switch, and the interrupt and clock backends; link the shared kernel. Goal:
   DragonFly's real boot messages up to "no root device".
2. **Single-user on a RAM disk.** Copy routines, system call path, signals,
   and enough cross-built userland for `/sbin/init` and a shell.
3. **SMP.** Secondary CPUs through PSCI, IPIs, TLB shootdown, and the
   memory-ordering audit.
4. **Real hardware.** GICv3, a device-tree bus, and drivers for one board
   (Raspberry Pi 4 or a Rockchip board with good documentation).
5. **Self-hosting.** Native toolchain, `buildworld` on arm64.
6. **Phones.** Each phone needs its own serial, storage, display, touch, power
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
- **Bring-up code that will be replaced:** `early_console.c` (its `kprintf`
  gives way to `kern/subr_prf.c`), the fixed 100Hz tick in `clock.c`, the
  single-pass device tree scanner, and the byte-loop `memcpy`/`memset` in
  `support.S`.
