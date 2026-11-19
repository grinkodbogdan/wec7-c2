# Linux personality

Flare runs unmodified **static Linux arm64 executables** on its own kernel.
The Linux system-call ABI is implemented in `kernel/src/linux/` on Flare
primitives; there is no Linux kernel anywhere (spec §8.1, Asterinas-style).

## What runs today

Verified on every `cargo xtask test`:

* **busybox 1.36 (Alpine, static musl)** as pid 1, running `/etc/rc`:
  command substitution, arithmetic, redirection, pipelines, `ls`, `cat`,
  `grep`, `wc`, `tr`, `sleep`, `uname`, reading `/proc/meminfo`, then an
  **interactive shell** on the console (line editing works).
* **A Rust `std` program** (`compat/linux/std-test`, target
  `aarch64-unknown-linux-musl`, static-pie): args, heap, `HashMap`, files,
  floating point, `thread::sleep`, `Command::output()` (fork + exec + pipes
  + wait).

## How it fits Flare

* A Linux process is a Flare component with a Linux `Process` attached and
  **no Flare capabilities**: it can only act through the Linux ABI.
* The root filesystem is an in-memory VFS unpacked from an initramfs that
  the kernel build packs from `target/flare-rootfs` (assembled by
  `cargo xtask build` from `compat/linux/rootfs`, busybox — downloaded from
  Alpine and checked against a pinned SHA-256 — and the test programs).
* The console TTY's input comes from the **user-space UART driver**
  (`TTY_INPUT`, which needs a `klog:write` capability); output goes through
  the kernel log, which the same driver prints. Native components and Linux
  processes share one console service, as spec §9 asks.
* The composition starts Linux with a `[linux]` section (`init`, `env`);
  `halt-when-finished = ["linux"]` powers off when init exits.

## Implemented

Processes: `clone` (fork and vfork semantics, copied address spaces),
`execve` (ELF static and static-pie, `#!` scripts), `wait4`, `exit_group`,
`kill`/`tgkill` (default actions), process groups and sessions, `uname`,
`sysinfo`, rlimits, credentials (everyone is root).

Memory: `brk`, `mmap` (anonymous and file-backed private), `munmap`,
`mprotect` (including `PROT_NONE`), on-demand stack growth to 8 MiB.

Files: `openat`, `read`/`write`/`readv`/`writev`/`pread64`/`pwrite64`,
`sendfile`, `lseek`, `getdents64`, `fstat`/`newfstatat`, `readlinkat`,
`faccessat`, `mkdirat`, `unlinkat`, `symlinkat`, `renameat`, `ftruncate`,
`dup`/`dup3`/`fcntl`, `pipe2`, `ppoll`, `getcwd`/`chdir`/`fchdir`.
Devices: `/dev/console`, `/dev/tty`, `/dev/null`, `/dev/zero`,
`/dev/urandom`. `/proc`: version, uptime, meminfo, cpuinfo, loadavg,
mounts, filesystems, cmdline, stat, `self/exe`.

TTY: termios (canonical and raw), echo, erase/kill, ^C (SIGINT to the
foreground group), ^D, window size, foreground process group.

Time: `clock_gettime`, `gettimeofday`, `nanosleep`, `clock_nanosleep`
(wall clock from the board's PL031 RTC).

Unimplemented calls return `ENOSYS` and are logged once
(`linux: unimplemented system call N`); the boot test fails if any appear.

## Not yet

* **Threads** (`clone` with `CLONE_THREAD`) — needs several threads per
  address space in the scheduler. Programs that spawn threads get `EAGAIN`.
* **Signal handlers** — signals with default actions work; installed
  handlers are recorded but not invoked.
* **Dynamic linking** — `PT_INTERP` executables are refused.
* Sockets, `epoll`, `pselect6`, `statx` (glibc and Rust fall back), shared
  file mappings, file permissions.

## Towards Android

Android needs everything above plus Android-specific contracts (spec §8.2):

1. **Bionic static executables** (NDK) — mostly the same ABI as above, plus
   the property area (`/dev/__properties__`).
2. **Threads and signal handlers** — required by bionic, ART and almost
   every Android service.
3. **Binder** (`/dev/binder` ioctl protocol) and **ashmem/memfd** — the IPC
   and shared-memory model of every Android service; a natural fit for
   Flare endpoints and memory capabilities.
4. **Dynamic linking** (`linker64`) — all of Android userspace is dynamic.
5. Then the AOSP userspace itself: `servicemanager`, `init`, ART + zygote,
   `system_server`, SurfaceFlinger with a Flare display service.

Steps 1–4 are kernel-ABI work in this repository. Step 5 means building and
booting AOSP userspace against that ABI.
