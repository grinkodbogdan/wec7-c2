# DragonFly BSD on arm64

An in-progress port of [DragonFly BSD](https://www.dragonflybsd.org/) to 64-bit
ARM (aarch64). DragonFly runs only on x86-64 upstream; this is the first part of
the work needed to change that.

**This is not a usable operating system yet.** The real DragonFly kernel boots
on emulated ARM64 machines and on a real Xiaomi Mi 5, mounts a root
filesystem from a ramdisk and runs user programs: an init that tests the kernel
and a small interactive shell, with a working terminal, signals, floating
point, a writable `/tmp` and, in the emulator, all CPU cores. What it cannot run
is real software, because DragonFly's C library and toolchain have not been
ported; the programs here use a few-hundred-line stand-in. It has no disk,
network, touch or GPU drivers. See [What works](#what-works) and
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
| `patches/ext2fs-4k-sectors.diff` | ext2fs reads its superblock as a whole 4KB block, for disks with 4KB sectors |
| `patches/hammer2-aarch64.diff` | HAMMER2's LZ4 and xxhash learn that aarch64 is 64-bit |
| `patches/libc-aarch64.diff` | Edits to shared libc and header files: TLS variant I in `gen/tls.c`, quad-precision `long double` in `<float.h>`, the thread library's spin hint |
| `lib/libc/aarch64/`, `lib/libc/thread/aarch64/` | The C library's machine-dependent part: system call stubs, `setjmp`, `getcontext`/`makecontext`, FP control (setjmp and FP files from FreeBSD/arm64) |
| `lib/csu/aarch64/` | Program startup files (`crt1`, `crti`/`crtn`, a minimal `crtbegin`/`crtend`) |
| `contrib/openbsd_libm/include/arm64/` | `<machine/ieee.h>` and `<machine/fenv.h>` for the math library |
| `world.sh` | Builds userland: sysroot headers, compiler-rt builtins, libc, startup files, programs, the C++ runtime, Mesa, FreeType and Arcan |
| `arcan/` | Arcan's `fbmem` video platform and the `term` and `hello` appls |
| `patches/arcan/`, `patches/llvm/` | Changes to Arcan and to LLVM's libc++ for DragonFly/arm64 |
| `tools/aarch64-dragonfly.cmake`, `tools/aarch64-dragonfly.meson.in` | Cross-build descriptions for CMake and Meson projects |
| `tools/qemu-shot.py` | Boots QEMU with a framebuffer, types commands, saves the screen as PNG |
| `user/` | Test userland: a mini C library (`libc.c`, not a port of DragonFly's), an `init` that tests the kernel, a small shell, and `hello.c`, a test of the real C library |
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

To build DragonFly's own C library and the test program that uses it
(also needs `bmake`, `rpcgen`, `flex` and `bison`):

```sh
./world.sh           # sysroot, compiler-rt, libc.a, startup files, hello
./build.sh ramdisk   # puts /bin/hello in the ramdisk
./build.sh run       # then type /bin/hello at the test shell
```

`world.sh` asks DragonFly's own makefiles (through `bmake -V`) which
sources and flags each library uses, and compiles them with clang for a
bare aarch64 ELF target with `__DragonFly__` defined.

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
at both a 2MB-aligned and an unaligned address, with 1, 2 and 4 CPU cores.
The checks `init` runs pass on all of them.

- **Multiple cores.** Secondary cores are started through PSCI `CPU_ON` and
  join the scheduler; cores send each other interrupts through the GIC. With
  four cores, four busy processes run in parallel (2.6x to 4x faster than one
  after another in QEMU) and four processes forking 100 children each at once
  run clean. `cpus=N` in the boot arguments limits the number of cores. The
  shell's `cpustat` shows each core's time and IPIs received.

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
| Software to run | DragonFly's real libc, static linking only; init and the shell still use the stand-in | The runtime linker (`rtld-elf`) and shared libraries, then DragonFly's own programs (`sh`, `ls`, ...) |
| User mode, remaining | Most of it works | Debugger support (`ptrace`, single step) |
| Storage | Ramdisk root, tmpfs; on the Mi 5 the UFS flash read-only (`qcom_ufs.c`), ext4 mounts read-only | Writes; interrupts and queued requests; bringing the controller up without LK (FreeBSD's ufshci plus Linux's Qualcomm PHY/clock code); a device-tree bus and bus DMA for other drivers |
| More than one CPU | Works in QEMU, untested on the phone | An audit of shared code that assumes x86's strong memory ordering; stopping the other cores on a panic (they keep running); a topology that knows the Mi 5's two clusters |
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

- **Thread-local storage layout.** Settled: arm64 uses the ABI's variant I.
  The thread pointer addresses the 16-byte ABI header (`tcb_dtv`,
  `tcb_pthread`) at the end of DragonFly's `struct tls_tcb`, and the other
  TCB fields sit below it. `tls_get_tcb()`/`tls_set_tcb()` hide the offset.
  The runtime linker still needs the same change.
- **Memory ordering.** x86 is strongly ordered and aarch64 is not. The atomic
  header uses the strongest ordering for every operation, which is safe, but
  shared kernel code that relies on plain loads and stores being ordered (it
  exists) still needs auditing. Nothing has shown up in testing so far.
- **`char` signedness.** `char` is unsigned on aarch64. Kernel code that
  assumes signed `char` compiles fine and misbehaves at run time.

## Roadmap

1. ~~Reach `mi_startup()` on one CPU and boot to the root prompt.~~ Done.
2. ~~A root filesystem and a first user process.~~ Done.
3. ~~Signals, floating point, a terminal and a shell.~~ Done, against a
   stand-in C library.
4. ~~Mi 5 first boot.~~ Done: boots to the shell with all checks passing.
5. ~~SMP.~~ Done in QEMU; the memory-ordering audit remains.
6. **Real userland.** ~~DragonFly's libc and thread library, static, with
   TLS~~: done in QEMU (`world.sh`, then `/bin/hello` in the test shell).
   Next: the runtime linker and shared libraries, then DragonFly's own
   programs replacing the test init and shell.
7. **Drivers.** A Linux driver compatibility layer, a device-tree bus, bus
   DMA, then the Mi 5's storage (UFS), touch, USB and display.
8. **Arcan**, on the real userland and the display, GPU and touch drivers.
9. **Self-hosting.** Native toolchain, `buildworld` on arm64.

## Xiaomi Mi 5

The Mi 5 (codename gemini, Snapdragon 820 / MSM8996) is the target phone.
It has been tried once. See [Hardware results](#hardware-results).

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
driver.

The image boots verbose by default (`-v`): before the usual messages it
prints lines starting with `boot:` that give the memory layout the bootloader
handed over, then one line per early boot stage. If the phone stops, the last
`boot:` line on the screen says where. `MI5_APPEND="..." ./build.sh mi5`
replaces the extra boot arguments; `MI5_APPEND=""` gives a quiet boot.

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

- Takes over the interrupt controller and switches off the timers before
  enabling interrupts, since the bootloader leaves both running.

### Hardware results

**First boot (10 November 2026), 3 GB model.** The bootloader accepted the
image, the kernel ran, found the display and drew the DragonFly copyright
notice over the fastboot screen in the right colours and font. It then
stopped.

Cause: the kernel enabled interrupts before it had set up the interrupt
controller. The bootloader leaves the controller enabled with an interrupt
pending (its timer), and with no controller set up that interrupt could not
be acknowledged, so the processor took it again forever. QEMU starts with a
quiet controller, which is why the emulator never showed it. The boot
argument `lkirq` now recreates the bootloader's state in QEMU: the first
image stops at exactly the same line, the fixed one boots to the shell.

Confirmed on hardware by that boot: image format and load addresses, the
watchdog stop, the device tree reader, finding the framebuffer, the pixel
format and the panel refresh.

**Second boot.** Nothing at all appeared over the fastboot screen. The
fix had moved interrupt controller setup ahead of the first message, and on
this chip the secure firmware forbids access to one of the controller's
registers (`GICR_WAKER`): touching it hangs the phone. Linux skips that
register on MSM8996 for the same reason (its
`FLAGS_WORKAROUND_GICR_WAKER_MSM8996`). The same image also switched off
the physical timer from EL1, which Qualcomm's hypervisor may trap. The
third image skips the register on this chip, leaves the physical timer
alone, and prints a `boot:` line before each early step, so a hang shows
where it happened.

**Third boot.** Everything up to the memory manager now works on the phone:
interrupt controller, timer (19.2 MHz), framebuffer (1080x1920, 24 bits per
pixel, found by reading the display controller), watchdog, the nine reserved
regions and the 209 MB of firmware memory. The kernel entered `mi_startup`
and stopped while sizing its table of RAM pages, printing a range whose start
was above its end. The bootloader lists the bank above 4 GB first
(`100000000-15eb50000`, then `80000000-e0000000`), and DragonFly's VM takes
the first free-memory entry as the lowest address and the last as the
highest. The free-memory list is now sorted. The boot argument `memrev`
presents QEMU's RAM in the same order: the old kernel faults at the same
step, the fixed one boots to the shell.

**Fourth boot (11 November 2026).** Boots all the way: 3051 MB of RAM found
and 2677 MB available after the kernel and reservations, the ramdisk mounted
as root, `init` started, all 14 checks passed, and the shell prompt shown on
the panel. Everything the kernel does so far works on the phone, on one core.

**Fifth boot, four cores.** The second core (`mpidr 1`, same cluster as the
boot core) started and reported in; then the boot stopped silently. The third
core (`mpidr 0x100`) is the first one in the Snapdragon 820's other cluster,
and neither of the messages a failed start prints appeared, so the boot core
never got past starting it. The emulator's cores are all in one cluster, so
it never exercised this. The iOS port for this chip starts all four cores
with the same firmware call; it also invalidates the new core's instruction
cache before turning its MMU on, and its started cores sit idle while the
next one powers up, where this kernel's spun hard on a lock. The sixth image
does both, and reports every step of starting each core (`smp:` lines: the
core's power state as the firmware sees it, the call, its result, and how
long the core took to report in).

**Sixth boot, four cores.** Stopped right after `smp: cpu1: CPU_ON`, this
time without the second core's lines either, and without the boot core
reporting the call's result. Two explanations fit both this and the fifth
boot: the firmware call does not return to the boot core, or the kernel kept
going and the panel stopped showing new frames. The second is plausible
because two cores were drawing and refreshing the panel at once, and a second
"start frame" command while a frame is in flight desynchronizes this panel.
The seventh image locks the console so only one core at a time draws or
refreshes the panel. It also has the first secondary core, while it waits,
report every few seconds what the boot core is doing (for example
`smp: cpu1 still waiting: boot cpu inside the PSCI CPU_ON call cpu2 for 5 s`),
which tells the two explanations apart. The boot argument `smp.stall=N`
reproduces a stuck call in QEMU.

**Seventh boot, four cores.** All four cores came online
(`SMP: active cpu mask 000000000000000f`), so the earlier hangs were the
panel desynchronizing under two cores, not the firmware. The boot ran
through the self-test to the shell, with one failure: one busy process
took 4.9 s, but four at once took 325 s instead of about 5 s. Work
landing on a secondary core runs dozens of times slower. QEMU does not
model this. The leading suspect is clock speed: the bootloader sets the
clock of the cluster it runs on, and without a cpufreq driver (Linux's
`clk-cpu-8996`) the other cluster may stay at its 19.2 MHz reference
clock. 19.2 MHz against roughly 1.3 GHz is close to the 66x measured.
An interrupt storm on the secondaries would look the same. The eighth
image tells them apart: each core measures its own clock with interrupts
off as it starts (`cpu2: running at about N MHz`), and the shell's
`cpustat` shows the measured speed and timer interrupts per core.

**Eighth boot.** Same result; the speed lines had scrolled away. The
framebuffer console does not scroll: it wraps to the top, so older
lines remain below the newest one. The ninth image prints each core's
clock and timer interrupt rate in the self-test, just before the
timing test, and skips that test (reporting a failure) when a core is
under a quarter of cpu0's speed.

**Ninth boot.** The speed lines settle it: cpu0 runs at about 1228 MHz,
cpu2 and cpu3 (the performance cluster, mpidr 0x100 and 0x101) at
19 MHz, the XO reference clock. The bootloader clocks only the cluster
it runs on. The tenth image programs the performance cluster's clock
the way Linux's `clk-cpu-8996` does at probe, before any voltage
change: GPLL0 (300 MHz) while the cluster PLL is set to 1036.8 MHz,
then the PLL. Linux's ACD leg is skipped (it needs the cluster's L2
set up from inside the cluster), and the power cluster is left at the
bootloader's setting. The boot argument `msm8996.perfcl=1` stops at
300 MHz, and `msm8996.perfcl=0` leaves the cluster alone.

**Tenth boot: four cores, full self-test passes.** Measured clocks
cpu0 1228, cpu1 1228, cpu2 1036, cpu3 1036 MHz; timer interrupts about
250 per second on every core. One busy process took 4938 ms, four at
once 5841 ms (3.3x). That is what the clocks predict: the four finish
when the two slower cores do, 4938 x 1228 / 1036 = 5853 ms. All checks
passed, and the shell runs.

**Eleventh boot: real userland.** The kernel self-test passed, then
every run of DragonFly's `sh` died with SIGILL ("unknown reason", the
CPU's undefined-instruction exception) at the first instruction of
`mmap`, a valid instruction. The instruction cache was stale: the
kernel synchronised it after loading program pages by invalidating by
the direct-map address, and the Kryo's virtually indexed instruction
cache keeps the user mapping's lines elsewhere. QEMU models no caches.
The twelfth image invalidates the whole instruction cache on CPUs whose
`CTR_EL0` reports a non-PIPT instruction cache, as Linux does, and a
SIGILL of this kind also prints the instruction word in memory.

**Twelfth boot.** Same crash; the instruction word in memory was
`0x00000000`. The cache was not at fault: the ramdisk was. LK loads the
boot image's ramdisk at 0x82a00000 and keeps its splash framebuffer at
0x83401000, so the 12MB ramdisk now ends inside the framebuffer, and
every line the console drew overwrote part of it (`/bin/sh` sat at
0x833b0000-0x83462508). The thirteenth image gives the display its own
buffer after the ramdisk, still inside LK's `cont_splash_mem`
reservation, and points the scanout pipe at it (`fb: moved off the
ramdisk` in the boot log). The instruction-cache change stays: it is
what Linux does for this kind of cache.

**Thirteenth boot: DragonFly's own userland on the phone.** The kernel
self-test passed on four cores, then init's demo ran DragonFly's `sh`,
`uname`, `sysctl`, `ls`, `df`, `ps`, `wc` and `tail` against the real C
library: 4 cpus, 2786 MB of memory, 37 programs, tmpfs on `/tmp`, and
`/bin/hello` reported `libc: all checks passed`. The shell then waits
at its prompt (there is no input device yet).

**Fourteenth boot: the phone's storage.** LK leaves the UFS controller
enabled (version 1.1, 32 slots, 64-bit addressing) with the link up and
its own transfer list running. The probe stopped the list, pointed it at
its own and read: a Toshiba THGLF2G9J8LBATRH, 64GB in 4KB blocks, as six
LUNs (56GB, 4MB, 4MB, 128MB, 4.4GB, 32MB), and LUN 0's GPT with 14
partitions (`ssd`, `persist`, `cache`, `userdata` and others). So a
driver can start with standard UFS commands and leave Qualcomm's clock,
regulator and PHY bring-up (Linux's `ufs-qcom`) for later, when the
kernel has to reinitialise the controller itself.

**Fifteenth boot: the flash as disks.** The probe became a read-only
driver (`qcom_ufs.c`): each LUN is `/dev/ufsN`, the kernel's slice code
found a GPT on all six, and LUN 0's 14 partitions appeared as
`ufs0s0`-`ufs0s13`. LBA 1 read back as `EFI PART`, and `dd` read 64MB in
0.9 s (74.5 MB/s) polled, one request at a time, through a bounce
buffer. The sixteenth image adds ext2fs (ext4 read-only, tested in QEMU
through `vn`) and mounts Android's `persist` and `cache` read-only.

**Sixteenth boot.** Both mounts failed: ext2fs read the superblock as
the 1KB at byte 1024, and the disk layer refused a transfer smaller than
the flash's 4KB sector (`dscheck: b_bcount 1024 is not on a sector
boundary`). `patches/ext2fs-4k-sectors.diff` reads the first 4KB of the
device instead and takes the superblock from inside it.

### HAMMER2

HAMMER2 builds into the arm64 kernel unchanged except for one bug:
`hammer2_lz4.c` chose its 64-bit mode only for `__x86_64__`, and its
32-bit mode keeps pointers in a hash table sized for 32-bit offsets,
which on a 64-bit CPU overruns the table. The first QEMU test formatted,
mounted and wrote a volume, then panicked in
`LZ4_compress64k_heap_limitedOutput` when the unmount flushed the
compressed data. `patches/hammer2-aarch64.diff` adds `__aarch64__` there,
in the userland copy, and in xxhash's alignment check. Since then, in
QEMU: `newfs_hammer2` on a 1.1GB `vn` volume, `mount_hammer2`, writing
all of `/bin` and `/sbin` plus 40MB of zeros (compressed away), unmount,
remount, and every file compares identical. The eighteenth image runs
the same test on the phone, on a sparse file in tmpfs (the flash driver
does not write yet).

**Eighteenth boot: ext4 and HAMMER2 on the phone.** Android's `persist`
(31.5MB, Xiaomi's sensor calibration files) and `cache` (252MB) mounted
read-only as ext4 from the UFS flash, and the HAMMER2 test passed on the
phone: a 1.1GB volume formatted, filled with 33 programs, remounted, and
every file identical.

**Nineteenth boot: buttons.** Power, volume down (PM8994 PON over SPMI),
volume up (PM8994 GPIO 2) and home (TLMM GPIO 34) all report presses and
releases (`qcom_pmic_keys.c`, polled every 20ms, SPMI reads after Linux's
`spmi-pmic-arb`). The twentieth image delivers them through evdev as
`/dev/input/event0` (the interface Arcan reads) and adds `keymenu`, which
init runs when buttons exist: volume up/down choose a command, power runs
it. The console now understands ANSI clear-screen and cursor-home, so the
menu redraws in place.

### Arcan

Arcan 0.7.0.1 (the last C release; later ones moved to Zig and Vulkan)
runs on DragonFly/arm64 with software rendering, so it needs no GPU
driver yet. The chain, all static:

- LLVM's libc++/libc++abi/libunwind (`world.sh cxx`), with DragonFly added
  next to FreeBSD in libc++ (`patches/llvm/libcxx-dragonfly.diff`).
- Mesa 24.0.9 as OSMesa with the softpipe renderer (`world.sh mesa`):
  OpenGL 3.3, GLSL 3.30.
- FreeType 2.13 (`world.sh freetype`); Lua 5.1 and SQLite from Arcan.
- A new Arcan video platform, `fbmem` (`arcan/src/platform/fbmem`): it
  renders through OSMesa at half the panel's resolution and copies each
  frame to `/dev/fb0`, a new kernel device (`fbmem.c`) that maps the
  console's framebuffer and pushes frames to the panel.
- Arcan's evdev input platform reads the phone's buttons from
  `/dev/input/event0`.
- `patches/arcan/arcan-dragonfly.diff`: the platform in the build, GL
  symbols resolved through the platform (static binaries have no
  `dlsym`), static shmif/tui/a12 libraries, DragonFly in evdev's BSD
  paths, and `ARCAN_FRAMESERVER_STDERR` for frameserver logs.

The kernel needed the pty driver (Arcan's terminal runs the shell on a
pty) and the LWKT token pool initialised at boot, as pc64 does; the pty
code was the first to use it. The ramdisk links `/var` to `/tmp/var` for
named semaphores, and now travels gzip-compressed (33MB, 13.8MB packed)
for the kernel to inflate.

The `term` appl (`arcan/appl/term`) is a full-screen terminal running
`sh` with a command bar: volume up/down choose a command, power types it
in, home sends Ctrl-C. In QEMU, `tools/qemu-shot.py` captures it: a typed
`uname -a; ls /; df -h /tmp; ps ax` runs and prints in the terminal.
On the phone, init starts it after the self-tests (`/etc/arcan.sh`).

**Twenty-first boot: Arcan on the phone.** Arcan started from init, the
terminal ran its autorun commands, and the command bar drew, but red and
blue were swapped (the 24-bit framebuffer LK sets up is blue-first) and
the buttons did nothing: Arcan's evdev platform files a device with only
four keys as a game device, so they arrive as digital events with the
key code in `subid`, which the appl did not handle. Both are fixed; the
button path is now tested in QEMU with `keys.fake=1` (the evdev device
without hardware) and `sysctl machdep.key_inject=CODE`, typed into
Arcan's own terminal from `/etc/arcan.autorun`.

### Touchscreen

`qcom_touch.c` drives the panel's Synaptics RMI4 touch controller (I2C
address 0x20) on BLSP2 QUP6, the bus the device tree calls `i2c12`. No
BSD has a driver for the QUP I2C controller or for RMI4, so both follow
Linux's register sequences (`i2c-qup.c`, version 1 tags, polled rather than
interrupt driven; `drivers/input/rmi4`), written afresh. A kernel thread
turns on the bus clocks and pins, finds the controller (powering it and
releasing its reset if it does not answer), reads the RMI4 function table,
configures F01, parses F12's register descriptors, and then samples the
finger objects at 60Hz. Touches go out through a multitouch evdev device,
`/dev/input/event1`, in panel pixels.

The `term` appl now has an on-screen keyboard above the command bar
(letters, digits, shift, a symbol layer, Ctrl, Tab, Enter, Delete), and a
tap on the bar chooses (left or right third) or runs (middle) a command.
Arcan's evdev platform needed one fix: the first touch after start came
through as a release. In QEMU, `touch.fake=1` provides the evdev device
without hardware and `sysctl machdep.touch_inject=X*65536+Y` taps;
`user/touchtest.sh` types `ls` and Enter on the keyboard that way.

**Twenty-fourth image: touch.** Untested on the phone. On start, Arcan's
terminal runs `dmesg | grep touch`, which shows how far the bring-up got.

**Twenty-fifth image: faster drawing.** On the phone touch works, but
the screen updated slowly: every frame is drawn by Mesa's software
renderer. Three changes, each measured in QEMU (`/tmp/arcan/fbmem.stats`
holds the same figures on the phone):

- The fbmem platform reads the finished frame out of Arcan's world render
  target instead of drawing it again as a full-screen textured quad
  (1400ms to 16ms per frame in QEMU).
- Only canvas rows that changed are converted and copied to `/dev/fb0`,
  and the new `FBMEM_FLUSH_ROWS` ioctl cleans only those rows from the
  cache.
- The on-screen keyboard is drawn once into its own texture and redrawn
  only when its layout changes, so the world draws one image instead of a
  hundred; the full-screen black backdrop is gone.

A frame takes about a third of the time it did. The real fix is the GPU.

The same round also fixed a kernel bug behind the occasional `sh`
segfaults: `cpu_fork()` copied the parent's saved thread pointer, which is
only updated when the parent is switched out, so a child forked before
that inherited a stale (zero) one and crashed on its first TLS access.

### Backlight, LEDs, vibrator, battery

The twenty-sixth image adds the phone's smaller parts, each behind a
sysctl under `hw.mi5`:

| sysctl | Part | Path |
|---|---|---|
| `backlight` (0-4095) | PMI8994 WLED | SPMI slave 3, 0xd800/0xd900 |
| `led` (0xRRGGBB) | notification LED: PMI8994 LPG channels through the TRILED | SPMI slave 3, 0xb100-0xb300, 0xd000 |
| `battery` (percent, read-only) | PMI8994 fuel gauge, monotonic state of charge | SPMI slave 2, 0x4009 |
| `charger` (read-only) | PMI8994 SMBCHG: none, present, charging, full | SPMI slave 2, 0x1000, 0x1300 |
| `vibrate` (milliseconds) | TI DRV2604 haptic driver, LRA, auto-calibrated at boot | I2C bus 9 at 0x5a, enable TLMM GPIO 93 |
| `keylight` (0-255) | TI LP5562, the capacitive keys' backlight | I2C bus 9 at 0x30, enable PM8994 GPIO 7 |

Underneath: `qcom_spmi.c` now writes as well as reads (Linux's
`spmi-pmic-arb` sequence: data, then command, on the channel registers),
and refuses writes to peripherals the arbiter's ownership table gives to
another execution environment. The QUP I2C code moved out of the touch
driver into `qcom_qup_i2c.c`, which serves both buses. The register
sequences follow Linux's `qcom-wled`, `leds-qcom-lpg`, `drv260x` and
`leds-lp5562` drivers and the Mi 5's downstream `qpnp-fg` and
`qpnp-smbcharger`; the board settings come from the upstream Mi 5 device
tree (`msm8996-xiaomi-gemini.dts`). The vibrator buzzes once and the key
lights come on when their drivers are ready.

GNU grep is now in the ramdisk (`world.sh` builds `gnu/usr.bin/grep`);
the twenty-fourth image's `dmesg | grep touch` had no grep to run.

**Twenty-sixth boot.** The touchscreen reports (`touch: ready`), the
backlight reads back (672 of 4095), the fuel gauge says 3% with a charger
present, and the DRV2604 answers (id 4), but its generic auto-calibration
fails (status 0x88) and the LP5562 does not answer. The twenty-seventh
image sets the vibrator up as Xiaomi's own driver does (fixed calibration
values, 240Hz LRA, rated 0x30 and overdrive 0x40, no auto-calibration)
and reports each step of the key light's enable line.

**Twenty-seventh boot.** The notification LED lights green for
0x00ff00, the backlight follows `hw.mi5.backlight`, and the fuel gauge
reads 13% and charging; but none of the GPU, vibrator or key-light
messages reached `dmesg`. DragonFly's `kprintf()` only *tries* the
console lock and drops the whole message when another cpu holds it;
with a slow framebuffer console and several driver threads printing at
once during boot, most of their lines were lost (the twenty-sixth boot
lost some too). `patches/kprintf-smp.diff` makes it wait, for a bounded
time and only outside interrupts.

**Twenty-ninth and thirtieth boots.** The vibrator works with Xiaomi's
settings. The GPU's rail turned out to be off and set to 1.00V (an FTS2.5
regulator, whose selector counts from 80mV, which the probe first decoded
as 1.27V and refused); the thirtieth image turned it on, and then the
GPU's memory-bus clock would not start because the GPU power domain was
still collapsed. The thirty-first image powers the domains before the
clocks, as Linux does. The key light's LP5562 still does not answer with
its enable line driven either way.

**Thirty-second boot.** The GPU is powered: vdd_gfx at 1.00V, both
power domains on (the GX domain once its core clock and root were forced
on first, as the Mi 5's kernel does), all five clocks running; but the
first read of a GPU register stalled that cpu, and the boot with it. The
thirty-third image also turns on `mmss_mmagic_ahb_clk` (the Mi 5's GPU
driver lists it as `alt_mem_iface_clk`), releases any GPU block reset
still asserted, and no longer reads GPU registers at boot:
`sysctl hw.mi5.gpu_read=1` does, one at a time, naming each first.

**Thirty-third boot.** The GPU answers: RBBM_STATUS 0 (idle),
RBBM_STATUS3 1, RBBM_INT_0_STATUS 0, CP_PROTECT_STATUS 0. Reading
RBBM_SECVID_TRUST_CNTL stalls the cpu: the GPU is in secure mode, and
only the secure world may change that. Linux takes it out with the
signed zap shader (`a530_zap.*`, loaded through the secure monitor's
PAS calls), which lives on the phone's own partitions. The thirty-fourth
image adds FAT (msdosfs and `mount_msdos`) and `/etc/gpufw.sh`, which
looks for the zap shader on every partition, read-only.

### GPU

`qcom_gpu.c` is the first step toward the Adreno 530: it reports the
state of the GPU's rail (vdd_gfx, PMI8994 S2), its two power domains
and clocks, then powers them up in Linux's order (rail, bus clocks, the
GPU domain, the GX domain and its I/O clamp, the core clocks) and only
then reads GPU and GPU-SMMU registers. `msm8996.gpu=1` reports without
powering anything; `0` skips it. The plan after that, as agreed: port
Linux's drm/msm Adreno driver (GPL, in its own directory), load the
microcode and the zap shader (from the phone's firmware partition, via
the secure monitor), then build Mesa's freedreno for Arcan.

### What is untested or missing

- **The second cluster's cores on the phone.** The seventh test image comes
  in two variants: all four cores, and `cpus=2` (the boot cluster only).
- **Input.** Buttons work; touch is new and untested on the phone; no USB.
- **Everything else.** USB, the GPU, Wi-Fi, audio, sensors, power
  management, charge control and the modem each need a driver.

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
- **Arcan adapted to this system.** Where it helps, change Arcan to make
  full use of HAMMER2 and the BSD base rather than treating them as a
  generic POSIX target: for example snapshots of application state and
  settings, per-application PFSes, and DragonFly's own interfaces in place
  of Linux ones.

Imported so far: from FreeBSD/arm64, libc's `setjmp` and FP-control files,
`crti.S`/`crtn.S`, and `<machine/ieeefp.h>`; from compiler-rt, the
builtins library. Taken from Linux as register-level protocols, not code:
the Kryo clock setup (`clk-cpu-8996`), the SPMI PMIC arbiter
(`spmi-pmic-arb`), the QUP I2C controller (`i2c-qup`), the Synaptics RMI4
touch protocol (`drivers/input/rmi4`), the PMI8994 WLED, LPG, fuel gauge
and charger, the DRV2604 and LP5562, and the GICv3 redistributor quirk.
