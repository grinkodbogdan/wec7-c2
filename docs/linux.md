# Linux personality

Flare runs unmodified **static Linux arm64 executables** on its own kernel.
The Linux system-call ABI is implemented in `kernel/src/linux/` on Flare
primitives; there is no Linux kernel anywhere (spec §8.1, Asterinas-style).

## What runs today

Verified on every `cargo xtask test`:

* **busybox 1.36 (Alpine, static musl)** as pid 1, running `/etc/rc`:
  command substitution, arithmetic, redirection, pipelines, `ls`, `cat`,
  `grep`, `wc`, `tr`, `sleep`, `timeout`, reading `/proc/meminfo`, then an
  **interactive shell** on the console (line editing works).
* **A Rust `std` program** (`compat/linux/std-test`, static-pie): args,
  heap, `HashMap`, files, floating point, `thread::sleep`,
  `Command::output()`.
* **The Linux ABI conformance suite** (`compat/linux/tests`, C programs
  taken from [QRT](https://github.com/grinkodbogdan/qrt)'s `tests/linux`,
  built with the musl SDK in `compat/linux/sdk`). All pass:
  * `threads` — pthreads, mutexes, condition variables, join, TLS (futex).
  * `signals` — handlers with and without `SA_SIGINFO`, masks and pending
    sets, SIGSEGV from read-only/PROT_NONE/NULL with `sigaltstack` and
    `siglongjmp`, SIGFPE, FP/SIMD state across a handler, `setitimer`,
    `alarm`, `SA_RESTART`, EINTR, `sigsuspend`, `sigtimedwait`, SIGCHLD,
    SIGPIPE, default actions, `pthread_kill`.
  * `memory` — an 8 GiB `PROT_NONE` reservation committed with `mprotect`,
    JIT (write then execute), no-execute data, memfd and `MAP_SHARED`
    across fork, `/dev/shm`, `mremap`, `MADV_DONTNEED`, a 64 MB realloc,
    `MAP_32BIT`, file mappings, 200 descriptors.
  * `events` — eventfd, timerfd, signalfd, epoll (one-shot, delete,
    timeouts), Unix sockets: socketpair with `SCM_RIGHTS`, datagram and
    seqpacket, a named server with listen/accept and `SO_PEERCRED`,
    abstract names, hang-up through epoll.
  * `procs` — fork, execve, wait, pipes, dup2, `posix_spawn`, `popen`,
    `O_CLOEXEC`, kill.

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

## Structure

`kernel/src/linux/`:

| File | What |
| --- | --- |
| `mod.rs` | tasks (threads) and processes (thread groups), exec, clone (fork, vfork, threads), exit, wait, futex, system-call dispatch |
| `mm.rs` | VMAs over a Flare address space, demand paging, shared memory objects (`MAP_SHARED`, memfd, `/dev/shm`), mmap/mprotect/mremap/madvise/brk |
| `signal.rs` | signal generation and delivery with Linux's arm64 `rt_sigframe` (layout as in QRT's `src/user/signal.c`), sigreturn, masks, alternate stack, itimers |
| `fs.rs` | open files: regular files, devices, pipes, eventfd, timerfd, signalfd, epoll, memfd; file system calls; ppoll |
| `net.rs` | Unix-domain sockets |
| `vfs.rs` | in-memory file system from the initramfs (copy-on-write over the kernel image) |
| `tty.rs` | console line discipline |
| `drm.rs` | `/dev/dri/card0` (KMS) and `/dev/fb0` over the display shared memory ([display.md](display.md)) |
| `input.rs` | `/dev/input/eventN` (evdev) fed by user-space input drivers |

Devices backed by Flare components:

* `/dev/dri/card0` and `/dev/fb0`: the panel, through the display driver's shared memory.
* `/dev/input/event*`: touch and keys, fed by input drivers.
* `/dev/fdb`: a `serial:` byte stream carried by the virtio-console driver; `fdbd` serves the debug bridge on it ([fdb.md](fdb.md)).

`/proc/<pid>` provides `stat`, `cmdline`, `comm`, `status` and `exe`, so
`ps`, `pidof` and `killall` work.

Blocking system calls are re-run when anything changes (the kernel never
blocks); a deliverable signal interrupts them with `EINTR` or restarts them
(`SA_RESTART`) by re-issuing the `svc`.

Unimplemented calls return `ENOSYS` and are logged once
(`linux: unimplemented system call N`); the boot test fails if any appear.

## Not yet

* **Dynamic linking** — `PT_INTERP` executables are refused.
* **Internet sockets** — no network stack yet (QRT has one in `src/net`
  that can be ported).
* `pselect6`, `statx` (glibc and Rust fall back), file permissions,
  job-control stops.

## Towards Android

Android needs everything above plus Android-specific contracts (spec §8.2):

1. **Bionic static executables** (NDK) — mostly the same ABI as above, plus
   the property area (`/dev/__properties__`).
2. ~~Threads and signal handlers~~ — done.
3. **Binder** (`/dev/binder` ioctl protocol) — the IPC of every Android
   service; a natural fit for Flare endpoints. (memfd, the modern ashmem,
   is done.)
4. **Dynamic linking** (`linker64`) — all of Android userspace is dynamic.
5. Then the AOSP userspace itself: `servicemanager`, `init`, ART + zygote,
   `system_server`, SurfaceFlinger with a Flare display service.

Steps 1–4 are kernel-ABI work in this repository. Step 5 is not the plan:
apps run from their APKs under `droid`, without AOSP userspace (see
[android.md](android.md)). The ABI work above serves apps' native code.
