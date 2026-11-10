# DragonFly BSD on arm64

An in-progress port of [DragonFly BSD](https://www.dragonflybsd.org/) to 64-bit
ARM (aarch64). DragonFly runs only on x86-64 upstream; this is the first part of
the work needed to change that.

**This is not a usable operating system yet.** The real DragonFly kernel boots
on an emulated ARM64 machine, mounts a root filesystem from a ramdisk and runs
user programs: an init that tests the kernel and a small interactive shell,
with a working terminal, signals, floating point and a writable `/tmp`. What it
cannot run is real software, because DragonFly's C library and toolchain have
not been ported; the programs here use a few-hundred-line stand-in. It also
uses one CPU only, has no disk, network or display drivers, and has never run
on real hardware. See [What works](#what-works) and
[What is missing](#what-is-missing).

## Target hardware

The intended first phone is the **Xiaomi Mi 5** (Snapdragon 820, 64-bit).
Nothing has been tried on one yet. See [Xiaomi Mi 5](#xiaomi-mi-5) for what is
in place and what is not.

The TicWatch C2 that the rest of this repository is about **cannot** run this:
it is a 32-bit ARM device, and DragonFly has been 64-bit only since 2014.

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
| `user/` | Test userland: a mini C library (`libc.c`, not a port of DragonFly's), an `init` that tests the kernel, and a small shell |
| `build.sh` | Fetch, overlay, build the kernel and ramdisk, and run them in QEMU |
| `tools/mkbootimg.py` | Packs kernel, device tree and ramdisk into an Android boot image for `fastboot boot` |
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
...
md0: Preloaded image <initrd> 434176 bytes at 0xfffff80048000000
Mounting root from cd9660:md0
Mounting devfs

Hello from user mode on DragonFly/aarch64

  ok      write to .data
  ok      mmap anonymous memory
  ok      demand-zero page reads 0
  ok      page fault on write
  ok      fork, child exit status 7
  ok      copy-on-write after fork
pid 3 (init): data abort (user) at pc 0x4006cc, addr 0x8, signal 11
pid 3 (init), uid 0: exited on signal 11
  ok      child killed by SIGSEGV
  ok      signal handler ran
  ok      handler could use floating point
  ok      floating point intact after handler
  ok      floating point across context switches
  ok      create a file on tmpfs
  ok      read it back
  ok      remove it
All checks passed.

DragonFly/arm64 test shell.  Type help for commands.
# uname
DragonFly 94448f62 aarch64 (ARM Cortex-A72)
# echo hello > /tmp/a
# cat /tmp/a
hello
# spin
spinning; press ^C to stop
^C[killed by signal 2]
# poweroff
```

The SIGSEGV line is part of the test: a child process dereferences a bad
pointer on purpose and the parent checks that it was killed. `help` lists the
shell's commands; `/sbin/init` re-runs the checks; `poweroff` shuts the kernel
down cleanly and exits QEMU.

Boot arguments work as on other platforms: `QEMU_APPEND="-v"` for verbose
boot, `-s` for single user, `name=value` for loader tunables, and `selftest`
to run the atomic-primitive checks. Other knobs: `QEMU_CPU=max`,
`QEMU_MEM=3G`, `QEMU_EXTRA="-M virt,virtualization=on"` (enter at EL2),
`QEMU_EXTRA="-M virt,gic-version=3"` (GICv3).
Without a ramdisk (`vfs.root.mountfrom` unset) the kernel stops at the
`mountroot>` prompt.

## What works

Tested under QEMU 8.2 `virt` with Cortex-A53, Cortex-A72 and `max` CPU models,
256MB to 3GB of RAM, GICv2 and GICv3, entered at both EL1 and EL2, and loaded
at both a 2MB-aligned and an unaligned address. One CPU only. The 14 checks
`init` runs pass on all of them.

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
- **Interrupts and time.** GICv2 and GICv3 dispatch that honours critical sections, soft
  interrupts, and the ARM generic timer as both the system timebase and the
  scheduler's interrupt source. Uptime advances and timeouts fire.
- **Console.** DragonFly's own `kprintf` and console layer on a PL011 serial
  port, on a firmware framebuffer, or both at once.
- **Root filesystem.** A ramdisk handed over the Linux way (QEMU `-initrd`, or
  the ramdisk slot of an Android boot image) becomes memory disk `md0` and is
  mounted as an ISO 9660 root.
- **User mode.** Programs are loaded by the standard ELF loader and run at
  EL0: system calls, page faults, fork with copy-on-write, exec, exit status.
- **Signals.** Handlers run on the user stack and return through
  `sigreturn`; processes without a handler are killed.
- **Floating point.** FP/SIMD registers are saved and restored across context
  switches, fork, exec and signal handlers.
- **Terminal.** The serial port is a real tty: line editing, echo, and
  Ctrl-C/Ctrl-Z delivered as signals to the foreground job.
- **Writable storage in memory.** tmpfs mounts on `/tmp`.
- **Reboot and power-off** through PSCI, including a clean shutdown requested
  from user mode.

## What is missing

Roughly in the order it blocks progress:

| Area | State | What it takes |
|---|---|---|
| Software to run | A stand-in C library and two test programs | A cross toolchain for `aarch64-dragonfly`, then DragonFly's libc, runtime linker and thread library, all of which have per-CPU assembly. This is the largest gap |
| User mode, remaining | Most of it works | Thread-local storage system calls (needed by the real libc and threads), debugger support (`ptrace`, single step) |
| Storage | Ramdisk root (read-only ISO 9660) and tmpfs | A device-tree bus so drivers can attach, bus DMA (`stubs.c` fails every request), then real disk drivers and a persistent filesystem |
| More than one CPU | Not started | Starting secondary CPUs through PSCI, inter-processor interrupts, and an audit of shared code that assumes x86's strong memory ordering |
| Interrupt controllers | GICv2 and GICv3, boot CPU only | Per-CPU setup for secondary CPUs, MSI (GICv2m or the GICv3 ITS) |
| pmap performance | Correct but slow | One global lock, no address-space IDs (every process switch flushes the TLB), no large pages, dirty/accessed tracking by over-approximation |
| Kernel modules | Cannot load | Relocation processing and a kernel symbol table |
| Debugging | None | The in-kernel debugger backend, crash dumps, `ptrace` register access |
| Build system | Bypassed | Teaching `config(8)` and the kernel makefiles about arm64 |
| Boot loader | QEMU `-kernel` only | A UEFI or U-Boot loader that can read HAMMER2 and pass modules |
| Real hardware | Untested | Nothing has run outside QEMU. Serial console only on a PL011; peripherals assumed below 1GB |

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
2. ~~A root filesystem and a first user process.~~ Done.
3. ~~Signals, floating point, a terminal and a shell.~~ Done, against a
   stand-in C library.
4. **Mi 5 first boot.** A test image builds (`./build.sh mi5`); what remains
   is trying it on the phone and fixing what that turns up. See below.
5. **Real userland.** Cross toolchain, DragonFly's libc and runtime linker,
   thread-local storage. This is the largest step.
6. **SMP.** Secondary CPUs, IPIs, and the memory-ordering audit.
7. **Drivers.** A device-tree bus, bus DMA, then storage, USB and network.
8. **Self-hosting.** Native toolchain, `buildworld` on arm64.

## Xiaomi Mi 5

The Mi 5 (codename gemini, Snapdragon 820 / MSM8996) is the target phone.
**This kernel has not been run on one yet.** What follows is ready to try.

The phone-specific facts used here (where the bootloader loads things, the
watchdog it leaves running, how to find and refresh the display, the RAM log
area) come from the [rpiios](https://github.com/grinkodbogdan/rpiios) project's
Mi 5 port, which established them on a real phone, and from the phone's device
trees in LineageOS's kernel. The kernel's device tree reader was run against
those device trees on a PC and gives the expected answers.

### Build and boot

```sh
./build.sh build     # kernel and ramdisk
./build.sh mi5       # -> work/obj/dragonfly-mi5-boot.img (needs cpp and dtc)
fastboot boot work/obj/dragonfly-mi5-boot.img
```

`fastboot boot` loads the image into RAM for one boot and writes **nothing**
to the phone's storage. Holding the power button until the phone restarts
returns to Android. Hold volume down + power to reach fastboot mode.

If it works, the bootloader's logo is replaced by white-on-black boot text
ending in the 14 self-test lines, `All checks passed.` and a `#` prompt, as in
`screenshot-fbcons.png`. The prompt cannot be used: there is no touch or key
driver. `MI5_APPEND="..." ./build.sh mi5` adds boot arguments (`-v` for a
verbose boot).

### If the screen shows nothing

The kernel also writes its console output to the RAM log Android uses
(ramoops, at 0x92920000), which survives a restart that does not cut power.
To read it: hold power until the phone restarts, boot TWRP for gemini with
`fastboot boot twrp.img`, and read `/sys/fs/pstore/console-ramoops-0`
(`adb shell cat ...`). Whatever is there shows how far the kernel got.

### What the kernel does on this phone

- Stops the watchdog the bootloader leaves running, which would otherwise
  reset the phone seconds into boot.
- Finds the framebuffer the bootloader is displaying by reading the display
  controller, and refreshes the panel itself after each line of text (the
  Mi 5's panels do not refresh on their own).
- Keeps the six memory regions the device tree reserves out of use, and
  removes the three marked as firmware-only (209 MB) from its address space.
- Takes the timer frequency from the device tree if the firmware did not
  program it.
- Uses the GICv3 interrupt controller under the `soc` bus, PSCI over SMC,
  and both memory banks (3 GB and 4 GB models).

### What is untested or missing

- **Everything above on real hardware.** It works on emulated machines and on
  the phone's device tree, not on the phone.
- **The display path.** The register layout is taken from rpiios's working
  loader; this kernel's code for it has never driven a real panel. If the
  text is garbled rather than absent, the pixel format guess is wrong.
- **Input.** No touch, keys or USB.
- **Everything after first boot.** Storage (UFS), USB, touch, display
  control, power management, charging and the modem each need a driver for
  this chip. One CPU core is used out of four.

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

## Project decisions

- **Code from other BSDs.** Where writing a DragonFly-native version of
  something is not practical, adapt it from FreeBSD's ARM64 port, keeping its
  copyright notices. Candidates: the per-CPU parts of libc and the runtime
  linker, the device-tree bus, virtio and SD/MMC drivers.
- **Drivers from Linux.** Where no BSD has a driver (most of this phone's
  chip: display, GPU, storage, touch, power), port the Linux one. Two things
  follow from that. Linux drivers are GPL-2.0, so a kernel that includes them
  can only be distributed under the GPL; they should live in their own
  directory, as DragonFly's existing Linux-derived graphics drivers do
  (`sys/dev/drm`), so the BSD-licensed parts stay separable. And they are
  written against Linux's kernel interfaces, so each needs the compatibility
  layer DragonFly already has for its graphics drivers, extended to cover
  clocks, regulators, GPIO and the device tree.
- **Arcan as the display server.** The UI will be built on
  [Arcan](https://arcan-fe.com/). Arcan draws through the kernel's
  mode-setting interface and OpenGL ES, so it needs, in order: a real
  userland (libc, C++ runtime, Lua), the phone's display driver, the GPU
  driver with Mesa's freedreno, and touch input delivered as evdev events.

Nothing has been imported from another project yet: so far everything here
was either written for this port or already in DragonFly's tree.
