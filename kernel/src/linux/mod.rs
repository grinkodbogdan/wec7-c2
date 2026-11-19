//! Linux ABI personality.
//!
//! Runs unmodified static Linux arm64 executables (musl, glibc-static,
//! Rust std, busybox) directly on the Flare framekernel: the Linux system
//! call ABI is implemented here, on Flare primitives (address spaces,
//! frames, the scheduler) and Flare services (console input/output reach the
//! user-space console driver). There is no Linux kernel underneath.
//!
//! A Linux process is a Flare component with a [`Process`] attached and no
//! Flare capabilities; it can only act through the Linux system calls
//! implemented here.

pub mod abi;
pub mod tty;
pub mod vfs;

use crate::arch::mmu::phys_to_virt;
use crate::arch::timer;
use crate::kernel::{Kernel, State};
use crate::mm::vspace::{AddressSpace, Prot, USER_MIN};
use crate::mm::{frame, page_ceil, page_floor, PAGE_SIZE};
use crate::{boot, elf};
use abi::nr;
use abi::*;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use tty::Tty;
use vfs::{NodeRef, Vfs};

/// Top of the initial stack; the stack may grow down to `STACK_LIMIT`.
pub const STACK_TOP: u64 = 0x7f_ffff_0000;
pub const STACK_LIMIT: u64 = STACK_TOP - 8 * 1024 * 1024;
const STACK_INITIAL: u64 = 128 * 1024;
/// Anonymous mappings are placed downward from here.
const MMAP_TOP: u64 = 0x7f_0000_0000;
/// Load address of position-independent executables.
const PIE_BASE: u64 = 0x55_5555_0000;
const MAX_FDS: usize = 256;

pub fn proc_files() -> &'static [&'static str] {
    &["version", "uptime", "meminfo", "cpuinfo", "loadavg", "mounts", "filesystems", "cmdline", "stat"]
}

pub struct Pipe {
    buf: VecDeque<u8>,
    readers: usize,
    writers: usize,
}

pub enum FileKind {
    Tty,
    Null,
    Zero,
    Random,
    Reg(NodeRef),
    Dir(NodeRef),
    PipeR(Rc<RefCell<Pipe>>),
    PipeW(Rc<RefCell<Pipe>>),
    /// Generated content (`/proc` files).
    Mem(Vec<u8>),
}

pub struct OpenFile {
    kind: FileKind,
    pos: u64,
    flags: u32,
    path: String,
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        match &self.kind {
            FileKind::PipeR(p) => p.borrow_mut().readers -= 1,
            FileKind::PipeW(p) => p.borrow_mut().writers -= 1,
            _ => {}
        }
    }
}

type FileRef = Rc<RefCell<OpenFile>>;

#[derive(Clone)]
pub struct Fd {
    file: FileRef,
    cloexec: bool,
}

#[derive(Clone, Copy, Default)]
struct SigAction {
    handler: u64,
    flags: u64,
    mask: u64,
}

pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pgid: u32,
    sid: u32,
    fds: Vec<Option<Fd>>,
    cwd: String,
    exe: String,
    brk_start: u64,
    brk: u64,
    mmap_top: u64,
    /// Address ranges reserved with PROT_NONE (mapped on mprotect).
    reserved: Vec<(u64, u64)>,
    sigactions: [SigAction; 65],
    sigmask: u64,
    /// Wait status once the process has exited (a zombie until reaped).
    pub status: Option<i32>,
    vfork_child: Option<u32>,
    execed: bool,
    sleep_until: Option<u64>,
    umask: u32,
    clear_tid: u64,
}

impl Process {
    fn new(pid: u32, ppid: u32) -> Process {
        Process {
            pid,
            ppid,
            pgid: pid,
            sid: pid,
            fds: Vec::new(),
            cwd: "/".to_string(),
            exe: String::new(),
            brk_start: 0,
            brk: 0,
            mmap_top: MMAP_TOP,
            reserved: Vec::new(),
            sigactions: [SigAction::default(); 65],
            sigmask: 0,
            status: None,
            vfork_child: None,
            execed: false,
            sleep_until: None,
            umask: 0o022,
            clear_tid: 0,
        }
    }

    fn fork(&self, pid: u32) -> Process {
        Process {
            pid,
            ppid: self.pid,
            pgid: self.pgid,
            sid: self.sid,
            fds: self.fds.clone(),
            cwd: self.cwd.clone(),
            exe: self.exe.clone(),
            brk_start: self.brk_start,
            brk: self.brk,
            mmap_top: self.mmap_top,
            reserved: self.reserved.clone(),
            sigactions: self.sigactions,
            sigmask: self.sigmask,
            status: None,
            vfork_child: None,
            execed: false,
            sleep_until: None,
            umask: self.umask,
            clear_tid: 0,
        }
    }

    fn install_fd(&mut self, file: FileRef, cloexec: bool, min: usize) -> Result<usize, i64> {
        let mut i = min;
        loop {
            if i >= MAX_FDS {
                return Err(EMFILE);
            }
            if i >= self.fds.len() {
                self.fds.resize(i + 1, None);
            }
            if self.fds[i].is_none() {
                self.fds[i] = Some(Fd { file, cloexec });
                return Ok(i);
            }
            i += 1;
        }
    }

    fn file(&self, fd: u64) -> Result<FileRef, i64> {
        self.fds.get(fd as usize).and_then(|f| f.as_ref()).map(|f| f.file.clone()).ok_or(EBADF)
    }
}

/// Personality-wide state.
pub struct Personality {
    pub vfs: Vfs,
    pub tty: Tty,
    next_pid: u32,
    pub init_finished: bool,
    rng: u64,
    epoch: u64,
    warned: Vec<u64>,
}

enum Out {
    Ret(i64),
    /// The process waits; the system call is re-run when something changes.
    Block,
    /// The process exited or its registers were replaced (exec).
    Gone,
}

fn err(e: i64) -> Out {
    Out::Ret(-e)
}

fn open_file(kind: FileKind, flags: u32, path: &str) -> FileRef {
    Rc::new(RefCell::new(OpenFile { kind, pos: 0, flags, path: path.to_string() }))
}

/// Seconds since 1970 at boot: the board's PL031 RTC if present.
fn rtc_epoch() -> u64 {
    match crate::platform::find("device", &["arm,pl031"]) {
        // SAFETY: PL031 data register, read through the kernel direct map.
        Some(d) => unsafe { core::ptr::read_volatile(phys_to_virt(d.regs[0].base) as *const u32) as u64 },
        None => 1_795_000_000,
    }
}

fn utsname() -> [u8; 390] {
    let mut b = [0u8; 390];
    for (i, s) in
        ["Linux", "flare", "6.6.0-flare", "#1 SMP Flare OS framekernel", "aarch64", "(none)"].iter().enumerate()
    {
        b[i * 65..i * 65 + s.len()].copy_from_slice(s.as_bytes());
    }
    b
}

fn stat_bytes(ino: u64, mode: u32, size: u64, rdev: u64, mtime: u64) -> [u8; 128] {
    let mut b = [0u8; 128];
    let mut put = |off: usize, v: &[u8]| b[off..off + v.len()].copy_from_slice(v);
    put(0, &1u64.to_le_bytes()); // st_dev
    put(8, &ino.to_le_bytes());
    put(16, &mode.to_le_bytes());
    put(20, &1u32.to_le_bytes()); // nlink
    put(32, &rdev.to_le_bytes());
    put(48, &size.to_le_bytes());
    put(56, &4096u32.to_le_bytes());
    put(64, &size.div_ceil(512).to_le_bytes());
    for off in [72, 88, 104] {
        put(off, &mtime.to_le_bytes());
    }
    b
}

fn node_stat(n: &NodeRef) -> [u8; 128] {
    let n = n.borrow();
    let size = if n.is_dir() { 4096 } else { n.data.len() as u64 };
    stat_bytes(n.ino, n.mode, size, n.rdev, n.mtime)
}

fn default_terminates(sig: u64) -> bool {
    !matches!(sig, SIGCHLD | SIGCONT | SIGURG | SIGWINCH | SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU)
}

fn cstr_bytes(v: &[u8]) -> Vec<u8> {
    let mut v = v.to_vec();
    v.push(0);
    v
}

impl Personality {
    pub fn new() -> Personality {
        let mut vfs = Vfs::new();
        match vfs.unpack(boot::INITRAMFS) {
            Ok(n) => kprintln!("linux: initramfs unpacked: {} entries", n),
            Err(e) => kprintln!("linux: initramfs damaged: {}", e),
        }
        let epoch = rtc_epoch();
        Personality {
            vfs,
            tty: Tty::new(),
            next_pid: 1,
            init_finished: false,
            rng: 0x9e37_79b9_7f4a_7c15 ^ timer::now_ns(),
            epoch,
            warned: Vec::new(),
        }
    }

    fn random(&mut self) -> u64 {
        let mut x = self.rng ^ timer::now_ns();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn proc_content(&self, name: &str, procs: usize) -> Option<Vec<u8>> {
        let up = timer::now_ns();
        let (total, used) = (frame::total() * 4, frame::in_use() * 4);
        let s = match name {
            "version" => "Linux version 6.6.0-flare (Flare OS framekernel, Linux ABI personality) #1 SMP\n".to_string(),
            "uptime" => format!("{}.{:02} 0.00\n", up / 1_000_000_000, (up / 10_000_000) % 100),
            "meminfo" => format!(
                "MemTotal:       {:8} kB\nMemFree:        {:8} kB\nMemAvailable:   {:8} kB\nBuffers:               0 kB\nCached:                0 kB\nSwapTotal:             0 kB\nSwapFree:              0 kB\n",
                total,
                total - used,
                total - used
            ),
            "cpuinfo" => "processor\t: 0\nBogoMIPS\t: 125.00\nFeatures\t: fp asimd evtstrm cpuid\nCPU implementer\t: 0x41\nCPU architecture: 8\nCPU part\t: 0xd03\n\n".to_string(),
            "loadavg" => format!("0.00 0.00 0.00 1/{} {}\n", procs, self.next_pid),
            "mounts" => "rootfs / rootfs rw 0 0\nproc /proc proc rw 0 0\ndevtmpfs /dev devtmpfs rw 0 0\n".to_string(),
            "filesystems" => "nodev\trootfs\nnodev\tproc\nnodev\tdevtmpfs\n".to_string(),
            "cmdline" => "console=ttyAMA0 init=/sbin/init\n".to_string(),
            "stat" => format!("cpu  0 0 0 {} 0 0 0 0 0 0\nbtime {}\nprocesses {}\n", up / 10_000_000, self.epoch, self.next_pid),
            _ => return None,
        };
        Some(s.into_bytes())
    }
}

impl Kernel {
    fn lx(&mut self) -> &mut Personality {
        self.linux.as_mut().expect("Linux personality not started")
    }

    fn proc(&mut self, c: usize) -> &mut Process {
        self.comps[c].linux.as_mut().expect("not a Linux process")
    }

    fn find_pid(&self, pid: u32) -> Option<usize> {
        self.comps.iter().position(|comp| !comp.free && comp.linux.as_ref().is_some_and(|p| p.pid == pid))
    }

    // ---------------------------------------------------------------
    // User memory helpers
    // ---------------------------------------------------------------

    fn uread(&self, c: usize, va: u64, len: usize) -> Result<Vec<u8>, i64> {
        self.copy_from_user(c, va, len).map_err(|_| EFAULT)
    }

    fn uwrite(&self, c: usize, va: u64, data: &[u8]) -> Result<(), i64> {
        self.copy_to_user(c, va, data).map_err(|_| EFAULT)
    }

    fn uread_u64(&self, c: usize, va: u64) -> Result<u64, i64> {
        Ok(u64::from_le_bytes(self.uread(c, va, 8)?.try_into().unwrap()))
    }

    fn ustr(&self, c: usize, mut va: u64) -> Result<String, i64> {
        let mut out = Vec::new();
        loop {
            let chunk = (PAGE_SIZE - va % PAGE_SIZE) as usize;
            let bytes = self.uread(c, va, chunk)?;
            if let Some(i) = bytes.iter().position(|&b| b == 0) {
                out.extend_from_slice(&bytes[..i]);
                break;
            }
            out.extend_from_slice(&bytes);
            if out.len() > 4096 {
                return Err(ENAMETOOLONG);
            }
            va += chunk as u64;
        }
        String::from_utf8(out).map_err(|_| EINVAL)
    }

    fn ustr_array(&self, c: usize, mut va: u64) -> Result<Vec<Vec<u8>>, i64> {
        let mut out = Vec::new();
        if va == 0 {
            return Ok(out);
        }
        loop {
            let p = self.uread_u64(c, va)?;
            if p == 0 {
                return Ok(out);
            }
            out.push(self.ustr(c, p)?.into_bytes());
            if out.len() > 4096 {
                return Err(EINVAL);
            }
            va += 8;
        }
    }

    // ---------------------------------------------------------------
    // Process lifecycle
    // ---------------------------------------------------------------

    /// Start the Linux personality and its init process (pid 1).
    pub(crate) fn linux_start(&mut self) {
        let Some(init) = boot::LINUX_INIT else { return };
        self.linux = Some(Personality::new());
        let c = self.alloc_comp("init");
        let pid = {
            let lx = self.lx();
            let pid = lx.next_pid;
            lx.next_pid += 1;
            pid
        };
        let mut p = Process::new(pid, 0);
        let tty = open_file(FileKind::Tty, O_RDWR, "/dev/console");
        for fd in 0..3 {
            let _ = p.install_fd(tty.clone(), false, fd);
        }
        self.comps[c].linux = Some(Box::new(p));
        let argv: Vec<Vec<u8>> = init.iter().map(|s| s.as_bytes().to_vec()).collect();
        let envp: Vec<Vec<u8>> = boot::LINUX_ENV.iter().map(|s| s.as_bytes().to_vec()).collect();
        match self.linux_exec(c, init[0], argv, envp) {
            Ok(()) => {
                kprintln!("linux: started init (pid 1): {}", init.join(" "));
                self.make_ready(c);
            }
            Err(e) => {
                kprintln!("linux: cannot start init {}: errno {}", init[0], e);
                self.comps[c].free = true;
                self.comps[c].linux = None;
                self.lx().init_finished = true;
            }
        }
    }

    /// Replace process `c`'s image (execve).
    fn linux_exec(&mut self, c: usize, path: &str, mut argv: Vec<Vec<u8>>, envp: Vec<Vec<u8>>) -> Result<(), i64> {
        let cwd = self.proc(c).cwd.clone();
        let mut path = path.to_string();
        let mut image;
        let mut depth = 0;
        loop {
            let r = self.lx().vfs.lookup(&cwd, &path, true)?;
            let n = r.node.borrow();
            if !n.is_reg() {
                return Err(EACCES);
            }
            if n.mode & 0o111 == 0 {
                return Err(EACCES);
            }
            image = n.data.clone();
            drop(n);
            if image.starts_with(b"#!") && depth < 4 {
                depth += 1;
                let line_end = image.iter().position(|&b| b == b'\n').unwrap_or(image.len());
                let line = String::from_utf8_lossy(&image[2..line_end]).trim().to_string();
                let mut parts = line.splitn(2, [' ', '\t']);
                let interp = parts.next().unwrap_or("").to_string();
                let arg = parts.next().map(|s| s.trim().to_string());
                let mut new_argv = vec![interp.clone().into_bytes()];
                if let Some(a) = arg.filter(|a| !a.is_empty()) {
                    new_argv.push(a.into_bytes());
                }
                new_argv.push(path.clone().into_bytes());
                new_argv.extend(argv.into_iter().skip(1));
                argv = new_argv;
                path = interp;
                continue;
            }
            if !elf::is_elf(&image) {
                return Err(ENOEXEC);
            }
            break;
        }

        let asid = self.free_asids.pop().ok_or(ENOMEM)?;
        let Some(mut vs) = AddressSpace::new(asid) else {
            self.free_asids.push(asid);
            return Err(ENOMEM);
        };
        let info = match elf::load(&image, &mut vs, USER_MIN..STACK_LIMIT, PIE_BASE) {
            Ok(i) => i,
            Err(e) => {
                kprintln!("linux: exec {}: {}", path, e);
                vs.destroy();
                self.free_asids.push(asid);
                return Err(ENOEXEC);
            }
        };
        // Initial stack.
        let mut va = STACK_TOP - STACK_INITIAL;
        while va < STACK_TOP {
            if vs.map_new(va, Prot::RW).is_err() {
                vs.destroy();
                self.free_asids.push(asid);
                return Err(ENOMEM);
            }
            va += PAGE_SIZE;
        }
        let random: [u8; 16] = {
            let lx = self.lx();
            let (a, b) = (lx.random(), lx.random());
            let mut r = [0u8; 16];
            r[..8].copy_from_slice(&a.to_le_bytes());
            r[8..].copy_from_slice(&b.to_le_bytes());
            r
        };
        let mut top = STACK_TOP;
        let mut place = |vs: &mut AddressSpace, data: &[u8]| -> u64 {
            top -= data.len() as u64;
            vs_write(vs, top, data);
            top
        };
        let execfn = place(&mut vs, &cstr_bytes(path.as_bytes()));
        let platform = place(&mut vs, b"aarch64\0");
        let rand_at = place(&mut vs, &random);
        let argv_at: Vec<u64> = argv.iter().map(|a| place(&mut vs, &cstr_bytes(a))).collect();
        let envp_at: Vec<u64> = envp.iter().map(|e| place(&mut vs, &cstr_bytes(e))).collect();
        let auxv: [(u64, u64); 16] = [
            (AT_PHDR, info.phdr),
            (AT_PHENT, info.phent),
            (AT_PHNUM, info.phnum),
            (AT_PAGESZ, PAGE_SIZE),
            (AT_BASE, 0),
            (AT_FLAGS, 0),
            (AT_ENTRY, info.entry),
            (AT_UID, 0),
            (AT_EUID, 0),
            (AT_GID, 0),
            (AT_EGID, 0),
            (AT_HWCAP, 0x3), // fp, asimd
            (AT_CLKTCK, 100),
            (AT_RANDOM, rand_at),
            (AT_PLATFORM, platform),
            (AT_EXECFN, execfn),
        ];
        let mut words: Vec<u64> = vec![argv.len() as u64];
        words.extend(&argv_at);
        words.push(0);
        words.extend(&envp_at);
        words.push(0);
        for (k, v) in auxv {
            words.push(k);
            words.push(v);
        }
        words.push(AT_SECURE);
        words.push(0);
        words.push(AT_NULL);
        words.push(0);
        let sp = (top - words.len() as u64 * 8) & !15;
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        vs_write(&mut vs, sp, &bytes);

        // Commit: replace the old image.
        if let Some(old) = self.comps[c].vspace.replace(vs) {
            self.free_asids.push(old.asid());
            old.destroy();
        }
        let name = path.rsplit('/').next().unwrap_or("?").to_string();
        let comp = &mut self.comps[c];
        comp.name = name;
        comp.frame = crate::arch::TrapFrame::zeroed();
        comp.frame.elr = info.entry;
        comp.frame.sp_el0 = sp;
        *comp.ext = Default::default();
        if self.ext_owner == Some(c) {
            self.ext_owner = None;
        }
        let p = self.proc(c);
        p.brk_start = info.end;
        p.brk = info.end;
        p.mmap_top = MMAP_TOP;
        p.reserved.clear();
        p.exe = vfs::absolute(&cwd, &path);
        p.execed = true;
        for fd in p.fds.iter_mut() {
            if fd.as_ref().is_some_and(|f| f.cloexec) {
                *fd = None;
            }
        }
        for a in p.sigactions.iter_mut() {
            if a.handler != SIG_IGN {
                *a = SigAction::default();
            }
        }
        Ok(())
    }

    /// Terminate process `c` with wait status `status`.
    pub(crate) fn linux_exit(&mut self, c: usize, status: i32) {
        let (pid, ppid) = {
            let p = self.proc(c);
            p.status = Some(status);
            p.fds.clear();
            (p.pid, p.ppid)
        };
        for comp in self.comps.iter_mut() {
            if let Some(p) = comp.linux.as_mut() {
                if !comp.free && p.ppid == pid && p.pid != pid {
                    p.ppid = 1;
                }
            }
        }
        self.teardown(c);
        if pid == 1 {
            let how = if status & 0x7f == 0 {
                format!("status {}", (status >> 8) & 0xff)
            } else {
                format!("signal {}", status & 0x7f)
            };
            kprintln!("linux: init exited ({})", how);
            self.lx().init_finished = true;
            self.check_halt();
        }
        // Nobody will reap orphans of a dead init, or children of init's parent.
        let parent_alive = ppid != 0 && self.find_pid(ppid).is_some_and(|i| self.comps[i].state != State::Dead);
        if !parent_alive {
            self.comps[c].free = true;
            self.comps[c].linux = None;
        }
    }

    fn linux_kill_one(&mut self, target: usize, sig: u64) {
        if sig == 0 {
            return;
        }
        let action = self.proc(target).sigactions[sig as usize].handler;
        if sig == SIGKILL || (action == SIG_DFL && default_terminates(sig)) {
            self.linux_exit(target, sig as i32);
        }
        // Handlers are not delivered yet; ignored and caught signals are dropped.
    }

    fn linux_kill(&mut self, c: usize, pid: i64, sig: u64) -> i64 {
        if sig > 64 {
            return -EINVAL;
        }
        let (me, my_pgid) = {
            let p = self.proc(c);
            (p.pid, p.pgid)
        };
        let targets: Vec<usize> = self
            .comps
            .iter()
            .enumerate()
            .filter(|(_, comp)| !comp.free && comp.state != State::Dead)
            .filter_map(|(i, comp)| comp.linux.as_ref().map(|p| (i, p)))
            .filter(|(_, p)| match pid {
                p0 if p0 > 0 => p.pid == p0 as u32,
                0 => p.pgid == my_pgid,
                -1 => p.pid != 1 && p.pid != me,
                g => p.pgid == (-g) as u32,
            })
            .map(|(i, _)| i)
            .collect();
        if targets.is_empty() {
            return -ESRCH;
        }
        for t in targets {
            self.linux_kill_one(t, sig);
        }
        0
    }

    /// Feed console input to the TTY (from the console driver).
    pub(crate) fn linux_tty_input(&mut self, bytes: &[u8]) {
        let Some(lx) = self.linux.as_mut() else { return };
        lx.tty.input(bytes);
        if core::mem::take(&mut lx.tty.interrupt) {
            let pg = lx.tty.fg_pgrp;
            let targets: Vec<usize> = self
                .comps
                .iter()
                .enumerate()
                .filter(|(_, comp)| !comp.free && comp.state != State::Dead)
                .filter(|(_, comp)| comp.linux.as_ref().is_some_and(|p| p.pgid == pg && p.pid != 1))
                .map(|(i, _)| i)
                .collect();
            for t in targets {
                self.linux_kill_one(t, SIGINT);
            }
        }
    }

    /// A user fault in a Linux process. Returns `true` if it was resolved
    /// (stack growth), otherwise the process is killed.
    pub(crate) fn linux_fault(&mut self, c: usize, far: u64, esr: u64, pc: u64) {
        let ec = esr >> 26;
        let translation = (esr & 0x3f) >> 2 == 1;
        if ec == 0x24 && translation && (STACK_LIMIT..STACK_TOP).contains(&far) {
            let vs = self.comps[c].vspace.as_mut().unwrap();
            if vs.map_new(page_floor(far), Prot::RW).is_ok() {
                return;
            }
        }
        let sig = if ec == 0x00 { 4 } else { SIGSEGV };
        let (pid, name) = (self.proc(c).pid, self.comps[c].name.clone());
        kprintln!(
            "linux: pid {} ({}) killed by {}: esr {:#x} addr {:#x} pc {:#x}",
            pid,
            name,
            if sig == 4 { "SIGILL" } else { "SIGSEGV" },
            esr,
            far,
            pc
        );
        self.linux_exit(c, sig as i32);
    }

    // ---------------------------------------------------------------
    // System calls
    // ---------------------------------------------------------------

    pub(crate) fn linux_syscall(&mut self, c: usize) {
        match self.linux_dispatch(c) {
            Out::Ret(v) => self.comps[c].frame.x[0] = v as u64,
            Out::Block => self.comps[c].state = State::LinuxWait,
            Out::Gone => {}
        }
    }

    /// Re-run the system calls of blocked processes.
    pub(crate) fn linux_poll(&mut self) {
        if self.linux.is_none() {
            return;
        }
        for c in 0..self.comps.len() {
            if self.comps[c].state != State::LinuxWait || self.comps[c].free {
                continue;
            }
            match self.linux_dispatch(c) {
                Out::Ret(v) => {
                    self.comps[c].frame.x[0] = v as u64;
                    self.make_ready(c);
                }
                Out::Block => {}
                Out::Gone => {
                    if self.comps[c].state == State::LinuxWait {
                        self.make_ready(c);
                    }
                }
            }
        }
    }

    fn warn_unimplemented(&mut self, n: u64) {
        let lx = self.lx();
        if !lx.warned.contains(&n) {
            lx.warned.push(n);
            kprintln!("linux: unimplemented system call {} (returning ENOSYS)", n);
        }
    }

    fn base_dir(&mut self, c: usize, dirfd: u64) -> Result<String, i64> {
        if dirfd as i64 == AT_FDCWD {
            return Ok(self.proc(c).cwd.clone());
        }
        let f = self.proc(c).file(dirfd)?;
        let f = f.borrow();
        match f.kind {
            FileKind::Dir(_) => Ok(f.path.clone()),
            _ => Err(ENOTDIR),
        }
    }

    fn linux_dispatch(&mut self, c: usize) -> Out {
        let f = self.comps[c].frame;
        let a = [f.x[0], f.x[1], f.x[2], f.x[3], f.x[4], f.x[5]];
        let r = match f.x[8] {
            nr::READ => return self.sys_read(c, a[0], a[1], a[2]),
            nr::WRITE => self.sys_write(c, a[0], a[1], a[2]),
            nr::READV => return self.sys_readv(c, a[0], a[1], a[2]),
            nr::WRITEV => self.sys_writev(c, a[0], a[1], a[2]),
            nr::PREAD64 => self.sys_pread(c, a[0], a[1], a[2], a[3]),
            nr::PWRITE64 => self.sys_pwrite(c, a[0], a[1], a[2], a[3]),
            nr::OPENAT => self.sys_openat(c, a[0], a[1], a[2] as u32, a[3] as u32),
            nr::CLOSE => {
                let p = self.proc(c);
                match p.fds.get_mut(a[0] as usize) {
                    Some(slot @ Some(_)) => {
                        *slot = None;
                        Ok(0)
                    }
                    _ => Err(EBADF),
                }
            }
            nr::LSEEK => self.sys_lseek(c, a[0], a[1] as i64, a[2]),
            nr::GETDENTS64 => self.sys_getdents(c, a[0], a[1], a[2]),
            nr::FSTAT => self.sys_fstat(c, a[0], a[1]),
            nr::NEWFSTATAT => self.sys_fstatat(c, a[0], a[1], a[2], a[3]),
            nr::STATX => Err(ENOSYS),
            nr::FACCESSAT | nr::FACCESSAT2 => self.sys_faccessat(c, a[0], a[1]),
            nr::READLINKAT => self.sys_readlinkat(c, a[0], a[1], a[2], a[3]),
            nr::GETCWD => {
                let cwd = cstr_bytes(self.proc(c).cwd.as_bytes());
                if (a[1] as usize) < cwd.len() {
                    Err(ERANGE)
                } else {
                    self.uwrite(c, a[0], &cwd).map(|_| cwd.len() as i64)
                }
            }
            nr::CHDIR => self.sys_chdir(c, a[0]),
            nr::FCHDIR => self.base_dir(c, a[0]).map(|d| {
                self.proc(c).cwd = d;
                0
            }),
            nr::MKDIRAT => self.sys_path2(c, a[0], a[1], |k, cwd, p| k.lx().vfs.mkdir(cwd, p, a[2] as u32)),
            nr::UNLINKAT => {
                self.sys_path2(c, a[0], a[1], |k, cwd, p| k.lx().vfs.unlink(cwd, p, a[2] & AT_REMOVEDIR != 0))
            }
            nr::SYMLINKAT => match self.ustr(c, a[0]) {
                Ok(target) => self.sys_path2(c, a[1], a[2], |k, cwd, p| k.lx().vfs.symlink(cwd, &target, p)),
                Err(e) => Err(e),
            },
            nr::RENAMEAT | nr::RENAMEAT2 => self.sys_rename(c, a[0], a[1], a[2], a[3]),
            nr::FTRUNCATE => self.sys_ftruncate(c, a[0], a[1]),
            nr::FCHMOD | nr::FCHMODAT | nr::FCHOWNAT | nr::FCHOWN | nr::UTIMENSAT => Ok(0),
            nr::DUP => {
                let p = self.proc(c);
                p.file(a[0]).and_then(|f| p.install_fd(f, false, 0)).map(|n| n as i64)
            }
            nr::DUP3 => self.sys_dup3(c, a[0], a[1], a[2] as u32),
            nr::FCNTL => self.sys_fcntl(c, a[0], a[1], a[2]),
            nr::IOCTL => self.sys_ioctl(c, a[0], a[1], a[2]),
            nr::PIPE2 => self.sys_pipe2(c, a[0], a[1] as u32),
            nr::PPOLL => return self.sys_ppoll(c, a[0], a[1], a[2]),
            nr::PSELECT6 => Err(ENOSYS),
            nr::SENDFILE => self.sys_sendfile(c, a[0], a[1], a[2], a[3]),
            nr::SYNC | nr::FSYNC | nr::FADVISE64 | nr::MSYNC | nr::MLOCK | nr::MADVISE => Ok(0),

            nr::BRK => Ok(self.sys_brk(c, a[0]) as i64),
            nr::MMAP => self.sys_mmap(c, a[0], a[1], a[2], a[3], a[4], a[5]),
            nr::MUNMAP => self.sys_munmap(c, a[0], a[1]),
            nr::MPROTECT => self.sys_mprotect(c, a[0], a[1], a[2]),
            nr::MREMAP => Err(ENOSYS),

            nr::CLONE => return self.sys_clone(c, a[0], a[1], a[2], a[3], a[4]),
            nr::CLONE3 => Err(ENOSYS),
            nr::EXECVE => return self.sys_execve(c, a[0], a[1], a[2]),
            nr::EXIT | nr::EXIT_GROUP => {
                self.linux_exit(c, ((a[0] & 0xff) << 8) as i32);
                return Out::Gone;
            }
            nr::WAIT4 => return self.sys_wait4(c, a[0] as i32 as i64, a[1], a[2]),
            nr::KILL => Ok(self.linux_kill(c, a[0] as i32 as i64, a[1])),
            nr::TKILL | nr::TGKILL => {
                let (pid, sig) = if f.x[8] == nr::TGKILL { (a[1], a[2]) } else { (a[0], a[1]) };
                Ok(self.linux_kill(c, pid as i32 as i64, sig))
            }
            nr::RT_SIGACTION => self.sys_sigaction(c, a[0], a[1], a[2]),
            nr::RT_SIGPROCMASK => self.sys_sigprocmask(c, a[0], a[1], a[2]),
            nr::RT_SIGSUSPEND => return Out::Block,
            nr::SIGALTSTACK | nr::RT_SIGRETURN => Ok(0),
            nr::NANOSLEEP | nr::CLOCK_NANOSLEEP => {
                let req = if f.x[8] == nr::NANOSLEEP { a[0] } else { a[2] };
                return self.sys_nanosleep(c, req, f.x[8] == nr::CLOCK_NANOSLEEP && a[1] & 1 != 0);
            }
            nr::SCHED_YIELD => {
                self.need_resched = true;
                Ok(0)
            }
            nr::GETPID => Ok(self.proc(c).pid as i64),
            nr::GETTID => Ok(self.proc(c).pid as i64),
            nr::GETPPID => Ok(self.proc(c).ppid as i64),
            nr::GETUID | nr::GETEUID | nr::GETGID | nr::GETEGID => Ok(0),
            nr::GETRESUID | nr::GETRESGID => {
                let z = [0u8; 4];
                self.uwrite(c, a[0], &z).and(self.uwrite(c, a[1], &z)).and(self.uwrite(c, a[2], &z)).map(|_| 0)
            }
            nr::SETUID | nr::SETGID | nr::SETREUID | nr::SETREGID | nr::SETRESUID | nr::SETRESGID | nr::SETGROUPS => {
                Ok(0)
            }
            nr::GETGROUPS => Ok(0),
            nr::SETPGID => self.sys_setpgid(c, a[0] as u32, a[1] as u32),
            nr::GETPGID => {
                let pid = if a[0] == 0 { self.proc(c).pid } else { a[0] as u32 };
                match self.find_pid(pid) {
                    Some(i) => Ok(self.comps[i].linux.as_ref().unwrap().pgid as i64),
                    None => Err(ESRCH),
                }
            }
            nr::GETSID => Ok(self.proc(c).sid as i64),
            nr::SETSID => {
                let p = self.proc(c);
                p.sid = p.pid;
                p.pgid = p.pid;
                Ok(p.pid as i64)
            }
            nr::SET_TID_ADDRESS => {
                let p = self.proc(c);
                p.clear_tid = a[0];
                Ok(p.pid as i64)
            }
            nr::SET_ROBUST_LIST | nr::PRCTL | nr::SETPRIORITY | nr::SETITIMER | nr::GETITIMER => Ok(0),
            nr::GETPRIORITY => Ok(20),
            nr::FUTEX => Ok(0),
            nr::RSEQ | nr::MEMBARRIER => Err(ENOSYS),
            nr::UMASK => {
                let p = self.proc(c);
                Ok(core::mem::replace(&mut p.umask, a[0] as u32 & 0o777) as i64)
            }
            nr::UNAME => self.uwrite(c, a[0], &utsname()).map(|_| 0),
            nr::SETHOSTNAME => Ok(0),
            nr::SYSINFO => self.sys_sysinfo(c, a[0]),
            nr::GETRLIMIT => self.sys_prlimit(c, a[0], 0, a[1]),
            nr::PRLIMIT64 => self.sys_prlimit(c, a[1], a[2], a[3]),
            nr::SETRLIMIT => Ok(0),
            nr::GETRUSAGE => self.uwrite(c, a[1], &[0u8; 144]).map(|_| 0),
            nr::TIMES => {
                let _ = if a[0] != 0 { self.uwrite(c, a[0], &[0u8; 32]) } else { Ok(()) };
                Ok((timer::now_ns() / 10_000_000) as i64)
            }
            nr::CLOCK_GETTIME => self.sys_clock_gettime(c, a[0], a[1]),
            nr::CLOCK_GETRES => {
                if a[1] != 0 {
                    let mut ts = [0u8; 16];
                    ts[8..].copy_from_slice(&1u64.to_le_bytes());
                    self.uwrite(c, a[1], &ts).map(|_| 0)
                } else {
                    Ok(0)
                }
            }
            nr::GETTIMEOFDAY => {
                let ns = timer::now_ns();
                let secs = self.lx().epoch + ns / 1_000_000_000;
                let mut tv = [0u8; 16];
                tv[..8].copy_from_slice(&secs.to_le_bytes());
                tv[8..].copy_from_slice(&((ns % 1_000_000_000) / 1000).to_le_bytes());
                if a[0] != 0 {
                    self.uwrite(c, a[0], &tv).map(|_| 0)
                } else {
                    Ok(0)
                }
            }
            nr::GETRANDOM => {
                let len = (a[1] as usize).min(4096);
                let bytes: Vec<u8> = (0..len.div_ceil(8)).flat_map(|_| self.lx().random().to_le_bytes()).collect();
                self.uwrite(c, a[0], &bytes[..len]).map(|_| len as i64)
            }
            nr::SCHED_GETAFFINITY => self.uwrite(c, a[2], &1u64.to_le_bytes()).map(|_| 8),
            nr::REBOOT => {
                if matches!(a[2], LINUX_REBOOT_CMD_POWER_OFF | LINUX_REBOOT_CMD_HALT | LINUX_REBOOT_CMD_RESTART) {
                    kprintln!("linux: reboot({:#x}) requested by pid {}", a[2], self.proc(c).pid);
                    self.halting = true;
                    Ok(0)
                } else {
                    Err(EINVAL)
                }
            }
            nr::SOCKET => Err(EAFNOSUPPORT),
            n => {
                self.warn_unimplemented(n);
                Err(ENOSYS)
            }
        };
        match r {
            Ok(v) => Out::Ret(v),
            Err(e) => err(e),
        }
    }

    // ----- files -----

    /// Read from a file. `None` means "would block".
    fn file_read(&mut self, file: &FileRef, max: usize) -> Result<Option<Vec<u8>>, i64> {
        let mut f = file.borrow_mut();
        let pos = f.pos as usize;
        match &f.kind {
            FileKind::Tty => {
                drop(f);
                Ok(self.lx().tty.read(max))
            }
            FileKind::Null => Ok(Some(Vec::new())),
            FileKind::Zero => Ok(Some(vec![0; max.min(65536)])),
            FileKind::Random => {
                drop(f);
                let n = max.min(4096);
                let lx = self.lx();
                Ok(Some((0..n).map(|_| lx.random() as u8).collect()))
            }
            FileKind::Reg(n) => {
                let data = n.borrow();
                let end = (pos + max).min(data.data.len());
                let out = if pos < end { data.data[pos..end].to_vec() } else { Vec::new() };
                drop(data);
                f.pos += out.len() as u64;
                Ok(Some(out))
            }
            FileKind::Mem(d) => {
                let end = (pos + max).min(d.len());
                let out = if pos < end { d[pos..end].to_vec() } else { Vec::new() };
                f.pos += out.len() as u64;
                Ok(Some(out))
            }
            FileKind::Dir(_) => Err(EISDIR),
            FileKind::PipeR(p) => {
                let mut p = p.borrow_mut();
                if p.buf.is_empty() {
                    return if p.writers == 0 { Ok(Some(Vec::new())) } else { Ok(None) };
                }
                let n = max.min(p.buf.len());
                Ok(Some(p.buf.drain(..n).collect()))
            }
            FileKind::PipeW(_) => Err(EBADF),
        }
    }

    fn file_write(&mut self, file: &FileRef, data: &[u8]) -> Result<usize, i64> {
        let mut f = file.borrow_mut();
        if f.flags & O_ACCMODE == 0 && !matches!(f.kind, FileKind::Tty | FileKind::PipeW(_)) {
            return Err(EBADF);
        }
        let append = f.flags & O_APPEND != 0;
        match &f.kind {
            FileKind::Tty => {
                drop(f);
                self.lx().tty.write(data);
                Ok(data.len())
            }
            FileKind::Null | FileKind::Zero | FileKind::Random => Ok(data.len()),
            FileKind::Reg(n) => {
                let n = n.clone();
                let mut node = n.borrow_mut();
                let pos = if append { node.data.len() } else { f.pos as usize };
                if node.data.len() < pos + data.len() {
                    node.data.resize(pos + data.len(), 0);
                }
                node.data[pos..pos + data.len()].copy_from_slice(data);
                f.pos = (pos + data.len()) as u64;
                Ok(data.len())
            }
            FileKind::PipeW(p) => {
                let mut p = p.borrow_mut();
                if p.readers == 0 {
                    return Err(EPIPE);
                }
                p.buf.extend(data);
                Ok(data.len())
            }
            FileKind::Dir(_) => Err(EISDIR),
            FileKind::Mem(_) | FileKind::PipeR(_) => Err(EBADF),
        }
    }

    fn sys_read(&mut self, c: usize, fd: u64, buf: u64, len: u64) -> Out {
        let file = match self.proc(c).file(fd) {
            Ok(f) => f,
            Err(e) => return err(e),
        };
        let nonblock = file.borrow().flags & O_NONBLOCK != 0;
        match self.file_read(&file, (len as usize).min(1 << 20)) {
            Ok(Some(data)) => match self.uwrite(c, buf, &data) {
                Ok(()) => Out::Ret(data.len() as i64),
                Err(e) => err(e),
            },
            Ok(None) if nonblock => err(EAGAIN),
            Ok(None) => Out::Block,
            Err(e) => err(e),
        }
    }

    fn sys_readv(&mut self, c: usize, fd: u64, iov: u64, cnt: u64) -> Out {
        // Read into the first non-empty buffer only (allowed short read).
        for i in 0..cnt.min(1024) {
            let (base, len) = match (self.uread_u64(c, iov + i * 16), self.uread_u64(c, iov + i * 16 + 8)) {
                (Ok(b), Ok(l)) => (b, l),
                _ => return err(EFAULT),
            };
            if len > 0 {
                return self.sys_read(c, fd, base, len);
            }
        }
        Out::Ret(0)
    }

    fn sys_write(&mut self, c: usize, fd: u64, buf: u64, len: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let data = self.uread(c, buf, (len as usize).min(1 << 20))?;
        self.file_write(&file, &data).map(|n| n as i64)
    }

    fn sys_writev(&mut self, c: usize, fd: u64, iov: u64, cnt: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let mut data = Vec::new();
        for i in 0..cnt.min(1024) {
            let base = self.uread_u64(c, iov + i * 16)?;
            let len = self.uread_u64(c, iov + i * 16 + 8)?;
            data.extend(self.uread(c, base, (len as usize).min(1 << 20))?);
        }
        self.file_write(&file, &data).map(|n| n as i64)
    }

    fn sys_pread(&mut self, c: usize, fd: u64, buf: u64, len: u64, off: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let saved = file.borrow().pos;
        file.borrow_mut().pos = off;
        let r = self.file_read(&file, len as usize);
        file.borrow_mut().pos = saved;
        let data = r?.ok_or(EAGAIN)?;
        self.uwrite(c, buf, &data).map(|_| data.len() as i64)
    }

    fn sys_pwrite(&mut self, c: usize, fd: u64, buf: u64, len: u64, off: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let data = self.uread(c, buf, len as usize)?;
        let saved = file.borrow().pos;
        file.borrow_mut().pos = off;
        let r = self.file_write(&file, &data);
        file.borrow_mut().pos = saved;
        r.map(|n| n as i64)
    }

    fn sys_sendfile(&mut self, c: usize, out_fd: u64, in_fd: u64, off_ptr: u64, count: u64) -> Result<i64, i64> {
        let (inf, outf) = {
            let p = self.proc(c);
            (p.file(in_fd)?, p.file(out_fd)?)
        };
        let saved = inf.borrow().pos;
        if off_ptr != 0 {
            inf.borrow_mut().pos = self.uread_u64(c, off_ptr)?;
        }
        let data = self.file_read(&inf, (count as usize).min(1 << 20))?.ok_or(EAGAIN)?;
        if off_ptr != 0 {
            let newoff = inf.borrow().pos;
            self.uwrite(c, off_ptr, &newoff.to_le_bytes())?;
            inf.borrow_mut().pos = saved;
        }
        self.file_write(&outf, &data).map(|n| n as i64)
    }

    fn sys_openat(&mut self, c: usize, dirfd: u64, path: u64, flags: u32, mode: u32) -> Result<i64, i64> {
        let path = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        let abs = vfs::absolute(&base, &path);
        let procs = self.comps.iter().filter(|x| x.linux.is_some() && !x.free).count();
        let umask = self.proc(c).umask;
        let lx = self.lx();
        if let Some(name) = abs.strip_prefix("/proc/") {
            if let Some(data) = lx.proc_content(name, procs) {
                let f = open_file(FileKind::Mem(data), flags, &abs);
                return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
            }
        }
        let r = if flags & O_CREAT != 0 {
            lx.vfs.create(&base, &path, mode & !umask, flags & O_EXCL != 0)?
        } else {
            lx.vfs.lookup(&base, &path, flags & O_NOFOLLOW == 0)?
        };
        let node = r.node.clone();
        let n = node.borrow();
        let kind = if n.is_dir() {
            if flags & O_ACCMODE != 0 {
                return Err(EISDIR);
            }
            FileKind::Dir(node.clone())
        } else if flags & O_DIRECTORY != 0 {
            return Err(ENOTDIR);
        } else if n.mode & S_IFMT == S_IFCHR {
            match n.rdev {
                vfs::DEV_NULL => FileKind::Null,
                vfs::DEV_ZERO => FileKind::Zero,
                vfs::DEV_URANDOM => FileKind::Random,
                _ => FileKind::Tty,
            }
        } else if n.is_reg() {
            FileKind::Reg(node.clone())
        } else {
            return Err(ELOOP);
        };
        drop(n);
        if flags & O_TRUNC != 0 && flags & O_ACCMODE != 0 {
            if let FileKind::Reg(n) = &kind {
                n.borrow_mut().data.clear();
            }
        }
        let f = open_file(kind, flags, &r.path);
        self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64)
    }

    fn sys_lseek(&mut self, c: usize, fd: u64, off: i64, whence: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let mut f = file.borrow_mut();
        let size = match &f.kind {
            FileKind::Reg(n) => n.borrow().data.len() as i64,
            FileKind::Mem(d) => d.len() as i64,
            FileKind::Dir(_) => 0,
            _ => return Err(ESPIPE),
        };
        let new = match whence {
            0 => off,
            1 => f.pos as i64 + off,
            2 => size + off,
            _ => return Err(EINVAL),
        };
        if new < 0 {
            return Err(EINVAL);
        }
        f.pos = new as u64;
        Ok(new)
    }

    fn sys_getdents(&mut self, c: usize, fd: u64, buf: u64, count: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let mut f = file.borrow_mut();
        let FileKind::Dir(node) = &f.kind else { return Err(ENOTDIR) };
        let dir = node.borrow();
        let mut entries: Vec<(String, u64, u8)> = vec![(".".into(), dir.ino, 4), ("..".into(), 1, 4)];
        for (name, child) in dir.children.iter() {
            let ch = child.borrow();
            let t = match ch.mode & S_IFMT {
                S_IFDIR => 4,
                S_IFLNK => 10,
                S_IFCHR => 2,
                S_IFIFO => 1,
                _ => 8,
            };
            entries.push((name.clone(), ch.ino, t));
        }
        drop(dir);
        let mut out = Vec::new();
        let mut i = f.pos as usize;
        while i < entries.len() {
            let (name, ino, t) = &entries[i];
            let reclen = (19 + name.len() + 1).div_ceil(8) * 8;
            if out.len() + reclen > count as usize {
                break;
            }
            let start = out.len();
            out.extend_from_slice(&ino.to_le_bytes());
            out.extend_from_slice(&((i + 1) as i64).to_le_bytes());
            out.extend_from_slice(&(reclen as u16).to_le_bytes());
            out.push(*t);
            out.extend_from_slice(name.as_bytes());
            out.resize(start + reclen, 0);
            i += 1;
        }
        if out.is_empty() && i < entries.len() {
            return Err(EINVAL);
        }
        f.pos = i as u64;
        drop(f);
        self.uwrite(c, buf, &out).map(|_| out.len() as i64)
    }

    fn fd_stat(&mut self, c: usize, fd: u64) -> Result<[u8; 128], i64> {
        let file = self.proc(c).file(fd)?;
        let f = file.borrow();
        Ok(match &f.kind {
            FileKind::Reg(n) | FileKind::Dir(n) => node_stat(n),
            FileKind::Tty => stat_bytes(3, S_IFCHR | 0o620, 0, vfs::DEV_CONSOLE, 0),
            FileKind::Null => stat_bytes(4, S_IFCHR | 0o666, 0, vfs::DEV_NULL, 0),
            FileKind::Zero => stat_bytes(5, S_IFCHR | 0o666, 0, vfs::DEV_ZERO, 0),
            FileKind::Random => stat_bytes(6, S_IFCHR | 0o444, 0, vfs::DEV_URANDOM, 0),
            FileKind::PipeR(_) | FileKind::PipeW(_) => stat_bytes(7, S_IFIFO | 0o600, 0, 0, 0),
            FileKind::Mem(d) => stat_bytes(8, S_IFREG | 0o444, d.len() as u64, 0, 0),
        })
    }

    fn sys_fstat(&mut self, c: usize, fd: u64, buf: u64) -> Result<i64, i64> {
        let st = self.fd_stat(c, fd)?;
        self.uwrite(c, buf, &st).map(|_| 0)
    }

    fn sys_fstatat(&mut self, c: usize, dirfd: u64, path: u64, buf: u64, flags: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        if p.is_empty() && flags & AT_EMPTY_PATH != 0 {
            return self.sys_fstat(c, dirfd, buf);
        }
        let base = self.base_dir(c, dirfd)?;
        let r = self.lx().vfs.lookup(&base, &p, flags & AT_SYMLINK_NOFOLLOW == 0)?;
        let st = node_stat(&r.node);
        self.uwrite(c, buf, &st).map(|_| 0)
    }

    fn sys_faccessat(&mut self, c: usize, dirfd: u64, path: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        self.lx().vfs.lookup(&base, &p, true).map(|_| 0)
    }

    fn sys_readlinkat(&mut self, c: usize, dirfd: u64, path: u64, buf: u64, size: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        let abs = vfs::absolute(&base, &p);
        let target = if abs == "/proc/self/exe" {
            self.proc(c).exe.clone().into_bytes()
        } else {
            let r = self.lx().vfs.lookup(&base, &p, false)?;
            let n = r.node.borrow();
            if !n.is_symlink() {
                return Err(EINVAL);
            }
            n.data.clone()
        };
        let n = target.len().min(size as usize);
        self.uwrite(c, buf, &target[..n]).map(|_| n as i64)
    }

    fn sys_chdir(&mut self, c: usize, path: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let cwd = self.proc(c).cwd.clone();
        let r = self.lx().vfs.lookup(&cwd, &p, true)?;
        if !r.node.borrow().is_dir() {
            return Err(ENOTDIR);
        }
        self.proc(c).cwd = r.path;
        Ok(0)
    }

    fn sys_path2(
        &mut self,
        c: usize,
        dirfd: u64,
        path: u64,
        op: impl FnOnce(&mut Kernel, &str, &str) -> Result<(), i64>,
    ) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        op(self, &base, &p).map(|_| 0)
    }

    fn sys_rename(&mut self, c: usize, ofd: u64, op: u64, nfd: u64, np: u64) -> Result<i64, i64> {
        let (o, n) = (self.ustr(c, op)?, self.ustr(c, np)?);
        let (ob, nb) = (self.base_dir(c, ofd)?, self.base_dir(c, nfd)?);
        let to = vfs::absolute(&nb, &n);
        self.lx().vfs.rename(&ob, &o, &to).map(|_| 0)
    }

    fn sys_ftruncate(&mut self, c: usize, fd: u64, len: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let f = file.borrow();
        match &f.kind {
            FileKind::Reg(n) => {
                n.borrow_mut().data.resize(len as usize, 0);
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }

    fn sys_dup3(&mut self, c: usize, old: u64, new: u64, flags: u32) -> Result<i64, i64> {
        let p = self.proc(c);
        let f = p.file(old)?;
        if old == new {
            return Err(EINVAL);
        }
        if new as usize >= MAX_FDS {
            return Err(EBADF);
        }
        if p.fds.len() <= new as usize {
            p.fds.resize(new as usize + 1, None);
        }
        p.fds[new as usize] = Some(Fd { file: f, cloexec: flags & O_CLOEXEC != 0 });
        Ok(new as i64)
    }

    fn sys_fcntl(&mut self, c: usize, fd: u64, cmd: u64, arg: u64) -> Result<i64, i64> {
        let p = self.proc(c);
        let file = p.file(fd)?;
        match cmd {
            0 | 1030 => p.install_fd(file, cmd == 1030, arg as usize).map(|n| n as i64),
            1 => Ok(p.fds[fd as usize].as_ref().unwrap().cloexec as i64),
            2 => {
                p.fds[fd as usize].as_mut().unwrap().cloexec = arg & 1 != 0;
                Ok(0)
            }
            3 => Ok(file.borrow().flags as i64),
            4 => {
                let mut f = file.borrow_mut();
                f.flags = (f.flags & !(O_APPEND | O_NONBLOCK)) | (arg as u32 & (O_APPEND | O_NONBLOCK));
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    fn sys_ioctl(&mut self, c: usize, fd: u64, req: u64, arg: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        if !matches!(file.borrow().kind, FileKind::Tty) {
            if req == FIONBIO {
                return Ok(0);
            }
            return Err(ENOTTY);
        }
        match req {
            TCGETS => {
                let t = self.lx().tty.termios.to_bytes();
                self.uwrite(c, arg, &t).map(|_| 0)
            }
            TCSETS | TCSETSW | TCSETSF => {
                let b = self.uread(c, arg, tty::TERMIOS_SIZE)?;
                self.lx().tty.termios = tty::Termios::from_bytes(&b);
                Ok(0)
            }
            TIOCGWINSZ => {
                let t = &self.lx().tty;
                let mut ws = [0u8; 8];
                ws[0..2].copy_from_slice(&t.rows.to_le_bytes());
                ws[2..4].copy_from_slice(&t.cols.to_le_bytes());
                self.uwrite(c, arg, &ws).map(|_| 0)
            }
            TIOCSWINSZ => {
                let b = self.uread(c, arg, 4)?;
                let t = &mut self.lx().tty;
                t.rows = u16::from_le_bytes([b[0], b[1]]);
                t.cols = u16::from_le_bytes([b[2], b[3]]);
                Ok(0)
            }
            TIOCGPGRP => {
                let pg = self.lx().tty.fg_pgrp;
                self.uwrite(c, arg, &pg.to_le_bytes()).map(|_| 0)
            }
            TIOCSPGRP => {
                let b = self.uread(c, arg, 4)?;
                self.lx().tty.fg_pgrp = u32::from_le_bytes(b.try_into().unwrap());
                Ok(0)
            }
            FIONREAD => {
                let n = self.lx().tty.pending() as u32;
                self.uwrite(c, arg, &n.to_le_bytes()).map(|_| 0)
            }
            TIOCSCTTY | TIOCNOTTY | FIONBIO => Ok(0),
            _ => Err(EINVAL),
        }
    }

    fn sys_pipe2(&mut self, c: usize, fds: u64, flags: u32) -> Result<i64, i64> {
        let pipe = Rc::new(RefCell::new(Pipe { buf: VecDeque::new(), readers: 1, writers: 1 }));
        let r = open_file(FileKind::PipeR(pipe.clone()), flags & O_NONBLOCK, "pipe:");
        let w = open_file(FileKind::PipeW(pipe), O_WRONLY | (flags & O_NONBLOCK), "pipe:");
        let cloexec = flags & O_CLOEXEC != 0;
        let p = self.proc(c);
        let rfd = p.install_fd(r, cloexec, 0)?;
        let wfd = match p.install_fd(w, cloexec, 0) {
            Ok(n) => n,
            Err(e) => {
                p.fds[rfd] = None;
                return Err(e);
            }
        };
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&(rfd as u32).to_le_bytes());
        b[4..].copy_from_slice(&(wfd as u32).to_le_bytes());
        self.uwrite(c, fds, &b).map(|_| 0)
    }

    fn sys_ppoll(&mut self, c: usize, fds: u64, nfds: u64, tmo: u64) -> Out {
        let raw = match self.uread(c, fds, (nfds.min(1024) * 8) as usize) {
            Ok(r) => r,
            Err(e) => return err(e),
        };
        let mut out = raw.clone();
        let mut ready = 0;
        for i in 0..nfds.min(1024) as usize {
            let fd = i32::from_le_bytes(raw[i * 8..i * 8 + 4].try_into().unwrap());
            let events = u16::from_le_bytes([raw[i * 8 + 4], raw[i * 8 + 5]]);
            let rev = if fd < 0 {
                0
            } else {
                match self.proc(c).file(fd as u64) {
                    Err(_) => POLLNVAL,
                    Ok(f) => {
                        let f = f.borrow();
                        let r = match &f.kind {
                            FileKind::Tty => {
                                let readable = self.linux.as_ref().unwrap().tty.readable();
                                (if readable { POLLIN } else { 0 }) | POLLOUT
                            }
                            FileKind::PipeR(p) => {
                                let p = p.borrow();
                                (if !p.buf.is_empty() { POLLIN } else { 0 })
                                    | (if p.writers == 0 { POLLHUP } else { 0 })
                            }
                            FileKind::PipeW(p) => {
                                if p.borrow().readers == 0 {
                                    POLLOUT | 0x8
                                } else {
                                    POLLOUT
                                }
                            }
                            _ => POLLIN | POLLOUT,
                        };
                        r & (events | POLLHUP | POLLNVAL | 0x8)
                    }
                }
            };
            out[i * 8 + 6..i * 8 + 8].copy_from_slice(&rev.to_le_bytes());
            if rev != 0 {
                ready += 1;
            }
        }
        let timeout_ns = if tmo == 0 {
            None
        } else {
            match (self.uread_u64(c, tmo), self.uread_u64(c, tmo + 8)) {
                (Ok(s), Ok(ns)) => Some(s.saturating_mul(1_000_000_000).saturating_add(ns)),
                _ => return err(EFAULT),
            }
        };
        let expired = match timeout_ns {
            Some(0) => true,
            Some(t) => {
                let now = timer::now_ns();
                let p = self.proc(c);
                let deadline = *p.sleep_until.get_or_insert(now + t);
                now >= deadline
            }
            None => false,
        };
        if ready > 0 || expired {
            self.proc(c).sleep_until = None;
            if let Err(e) = self.uwrite(c, fds, &out) {
                return err(e);
            }
            return Out::Ret(ready);
        }
        Out::Block
    }

    // ----- memory -----

    fn sys_brk(&mut self, c: usize, addr: u64) -> u64 {
        let (start, cur) = {
            let p = self.proc(c);
            (p.brk_start, p.brk)
        };
        if addr < start || addr >= STACK_LIMIT {
            return cur;
        }
        let (old_end, new_end) = (page_ceil(cur), page_ceil(addr));
        let vs = self.comps[c].vspace.as_mut().unwrap();
        if new_end > old_end {
            let mut va = old_end;
            while va < new_end {
                if !vs.is_mapped(va) && vs.map_new(va, Prot::RW).is_err() {
                    return cur;
                }
                va += PAGE_SIZE;
            }
        } else {
            let mut va = new_end;
            while va < old_end {
                vs.unmap(va);
                va += PAGE_SIZE;
            }
        }
        self.proc(c).brk = addr;
        addr
    }

    #[allow(clippy::too_many_arguments)]
    fn sys_mmap(
        &mut self,
        c: usize,
        addr: u64,
        len: u64,
        prot: u64,
        flags: u64,
        fd: u64,
        off: u64,
    ) -> Result<i64, i64> {
        if len == 0 {
            return Err(EINVAL);
        }
        let len = page_ceil(len);
        let fixed = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0;
        let va = if fixed {
            if !addr.is_multiple_of(PAGE_SIZE) || addr < USER_MIN || addr + len > STACK_LIMIT {
                return Err(EINVAL);
            }
            addr
        } else {
            let p = self.proc(c);
            let va = p.mmap_top.checked_sub(len).ok_or(ENOMEM)?;
            if va < p.brk_start + (1 << 30) {
                return Err(ENOMEM);
            }
            p.mmap_top = va;
            va
        };
        let data = if flags & MAP_ANONYMOUS == 0 {
            let file = self.proc(c).file(fd)?;
            let f = file.borrow();
            let src: Vec<u8> = match &f.kind {
                FileKind::Reg(n) => n.borrow().data.clone(),
                FileKind::Mem(d) => d.clone(),
                FileKind::Zero => Vec::new(),
                _ => return Err(EACCES),
            };
            let start = (off as usize).min(src.len());
            let end = (start + len as usize).min(src.len());
            Some(src[start..end].to_vec())
        } else {
            None
        };
        let vs = self.comps[c].vspace.as_mut().unwrap();
        let mut page = va;
        while page < va + len {
            vs.unmap(page);
            page += PAGE_SIZE;
        }
        if prot & (PROT_READ | PROT_WRITE | PROT_EXEC) == 0 {
            self.proc(c).reserved.push((va, va + len));
            return Ok(va as i64);
        }
        let mut page = va;
        while page < va + len {
            vs.map_new(page, Prot::RW).map_err(|_| ENOMEM)?;
            page += PAGE_SIZE;
        }
        if let Some(d) = data {
            vs_write(vs, va, &d);
        }
        let pr = Prot { none: false, write: prot & PROT_WRITE != 0, exec: prot & PROT_EXEC != 0 };
        if pr != Prot::RW {
            let mut page = va;
            while page < va + len {
                vs.protect(page, pr);
                if pr.exec {
                    if let Some(pa) = vs.lookup(page) {
                        crate::arch::sync_icache(phys_to_virt(pa), PAGE_SIZE);
                    }
                }
                page += PAGE_SIZE;
            }
        }
        Ok(va as i64)
    }

    fn sys_munmap(&mut self, c: usize, addr: u64, len: u64) -> Result<i64, i64> {
        if !addr.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        let end = addr + page_ceil(len);
        let vs = self.comps[c].vspace.as_mut().unwrap();
        let mut page = addr;
        while page < end {
            vs.unmap(page);
            page += PAGE_SIZE;
        }
        self.proc(c).reserved.retain(|&(s, e)| e <= addr || s >= end);
        Ok(0)
    }

    fn sys_mprotect(&mut self, c: usize, addr: u64, len: u64, prot: u64) -> Result<i64, i64> {
        if !addr.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        let end = addr + page_ceil(len);
        let pr = Prot {
            none: prot & (PROT_READ | PROT_WRITE | PROT_EXEC) == 0,
            write: prot & PROT_WRITE != 0,
            exec: prot & PROT_EXEC != 0,
        };
        let reserved = self.proc(c).reserved.clone();
        let vs = self.comps[c].vspace.as_mut().unwrap();
        let mut page = addr;
        while page < end {
            if !vs.is_mapped(page) {
                if !reserved.iter().any(|&(s, e)| (s..e).contains(&page)) {
                    return Err(ENOMEM);
                }
                if !pr.none {
                    vs.map_new(page, Prot::RW).map_err(|_| ENOMEM)?;
                    vs.protect(page, pr);
                }
            } else {
                vs.protect(page, pr);
            }
            page += PAGE_SIZE;
        }
        Ok(0)
    }

    // ----- processes -----

    fn sys_clone(&mut self, c: usize, flags: u64, stack: u64, ptid: u64, tls: u64, ctid: u64) -> Out {
        // A vfork parent resumes once the child has exec'd or exited.
        if let Some(child) = self.proc(c).vfork_child {
            let done = match self.find_pid(child) {
                None => true,
                Some(i) => self.comps[i].state == State::Dead || self.comps[i].linux.as_ref().unwrap().execed,
            };
            if !done {
                return Out::Block;
            }
            self.proc(c).vfork_child = None;
            return Out::Ret(child as i64);
        }
        if flags & CLONE_THREAD != 0 {
            self.warn_unimplemented(nr::CLONE | 0x1000_0000);
            return err(EAGAIN);
        }
        let Some(asid) = self.free_asids.pop() else { return err(EAGAIN) };
        self.save_ext(c);
        let Some(vs) = self.comps[c].vspace.as_ref().unwrap().duplicate(asid) else {
            self.free_asids.push(asid);
            return err(ENOMEM);
        };
        let pid = {
            let lx = self.lx();
            let pid = lx.next_pid;
            lx.next_pid += 1;
            pid
        };
        let child_proc = self.proc(c).fork(pid);
        let name = self.comps[c].name.clone();
        let n = self.alloc_comp(&name);
        let mut frame = self.comps[c].frame;
        frame.x[0] = 0;
        if stack != 0 {
            frame.sp_el0 = stack;
        }
        let mut ext = self.comps[c].ext.clone();
        if flags & CLONE_SETTLS != 0 {
            ext.tpidr = tls;
        }
        {
            let ch = &mut self.comps[n];
            ch.vspace = Some(vs);
            ch.frame = frame;
            ch.ext = ext;
            ch.linux = Some(Box::new(child_proc));
        }
        if flags & CLONE_CHILD_CLEARTID != 0 {
            self.proc(n).clear_tid = ctid;
        }
        if flags & CLONE_CHILD_SETTID != 0 {
            let _ = self.uwrite(n, ctid, &pid.to_le_bytes());
        }
        if flags & CLONE_PARENT_SETTID != 0 {
            let _ = self.uwrite(c, ptid, &pid.to_le_bytes());
        }
        self.make_ready(n);
        if flags & CLONE_VFORK != 0 {
            self.proc(c).vfork_child = Some(pid);
            return Out::Block;
        }
        Out::Ret(pid as i64)
    }

    fn sys_execve(&mut self, c: usize, path: u64, argv: u64, envp: u64) -> Out {
        let r = (|| -> Result<(), i64> {
            let p = self.ustr(c, path)?;
            let argv = self.ustr_array(c, argv)?;
            let envp = self.ustr_array(c, envp)?;
            self.linux_exec(c, &p, argv, envp)
        })();
        match r {
            Ok(()) => Out::Gone,
            Err(e) => err(e),
        }
    }

    fn sys_wait4(&mut self, c: usize, pid: i64, wstatus: u64, options: u64) -> Out {
        let (me, my_pgid) = {
            let p = self.proc(c);
            (p.pid, p.pgid)
        };
        let mut has_child = false;
        let mut found = None;
        for (i, comp) in self.comps.iter().enumerate() {
            let Some(p) = comp.linux.as_ref() else { continue };
            if comp.free || p.ppid != me {
                continue;
            }
            let matches = match pid {
                -1 => true,
                p0 if p0 > 0 => p.pid == p0 as u32,
                0 => p.pgid == my_pgid,
                g => p.pgid == (-g) as u32,
            };
            if !matches {
                continue;
            }
            has_child = true;
            if let Some(st) = p.status {
                found = Some((i, p.pid, st));
                break;
            }
        }
        match found {
            Some((i, cpid, st)) => {
                if wstatus != 0 {
                    if let Err(e) = self.uwrite(c, wstatus, &st.to_le_bytes()) {
                        return err(e);
                    }
                }
                self.comps[i].free = true;
                self.comps[i].linux = None;
                Out::Ret(cpid as i64)
            }
            None if !has_child => err(ECHILD),
            None if options & WNOHANG != 0 => Out::Ret(0),
            None => Out::Block,
        }
    }

    fn sys_setpgid(&mut self, c: usize, pid: u32, pgid: u32) -> Result<i64, i64> {
        let me = self.proc(c).pid;
        let target = if pid == 0 { c } else { self.find_pid(pid).ok_or(ESRCH)? };
        let p = self.comps[target].linux.as_mut().unwrap();
        if p.pid != me && p.ppid != me {
            return Err(ESRCH);
        }
        p.pgid = if pgid == 0 { p.pid } else { pgid };
        Ok(0)
    }

    fn sys_sigaction(&mut self, c: usize, sig: u64, act: u64, oact: u64) -> Result<i64, i64> {
        if sig == 0 || sig > 64 || ((sig == SIGKILL || sig == SIGSTOP) && act != 0) {
            return Err(EINVAL);
        }
        let old = self.proc(c).sigactions[sig as usize];
        if act != 0 {
            let b = self.uread(c, act, 32)?;
            let w = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
            self.proc(c).sigactions[sig as usize] = SigAction { handler: w(0), flags: w(1), mask: w(3) };
        }
        if oact != 0 {
            let mut b = [0u8; 32];
            b[0..8].copy_from_slice(&old.handler.to_le_bytes());
            b[8..16].copy_from_slice(&old.flags.to_le_bytes());
            b[24..32].copy_from_slice(&old.mask.to_le_bytes());
            self.uwrite(c, oact, &b)?;
        }
        Ok(0)
    }

    fn sys_sigprocmask(&mut self, c: usize, how: u64, set: u64, oset: u64) -> Result<i64, i64> {
        let old = self.proc(c).sigmask;
        if set != 0 {
            let s = self.uread_u64(c, set)?;
            let p = self.proc(c);
            p.sigmask = match how {
                0 => old | s,
                1 => old & !s,
                2 => s,
                _ => return Err(EINVAL),
            };
        }
        if oset != 0 {
            self.uwrite(c, oset, &old.to_le_bytes())?;
        }
        Ok(0)
    }

    fn sys_nanosleep(&mut self, c: usize, req: u64, absolute: bool) -> Out {
        let now = timer::now_ns();
        if self.proc(c).sleep_until.is_none() {
            let (s, ns) = match (self.uread_u64(c, req), self.uread_u64(c, req + 8)) {
                (Ok(s), Ok(ns)) => (s, ns),
                _ => return err(EFAULT),
            };
            let t = s.saturating_mul(1_000_000_000).saturating_add(ns);
            let deadline = if absolute { t } else { now.saturating_add(t) };
            self.proc(c).sleep_until = Some(deadline);
        }
        if now >= self.proc(c).sleep_until.unwrap() {
            self.proc(c).sleep_until = None;
            return Out::Ret(0);
        }
        Out::Block
    }

    fn sys_sysinfo(&mut self, c: usize, buf: u64) -> Result<i64, i64> {
        let mut b = [0u8; 112];
        let up = timer::now_ns() / 1_000_000_000;
        let total = frame::total() * PAGE_SIZE;
        let free = total - frame::in_use() * PAGE_SIZE;
        let procs = self.comps.iter().filter(|x| x.linux.is_some() && !x.free).count() as u16;
        b[0..8].copy_from_slice(&up.to_le_bytes());
        b[32..40].copy_from_slice(&total.to_le_bytes());
        b[40..48].copy_from_slice(&free.to_le_bytes());
        b[80..82].copy_from_slice(&procs.to_le_bytes());
        b[104..108].copy_from_slice(&1u32.to_le_bytes());
        self.uwrite(c, buf, &b).map(|_| 0)
    }

    fn sys_prlimit(&mut self, c: usize, res: u64, _new: u64, old: u64) -> Result<i64, i64> {
        if old != 0 {
            let (cur, max) = match res {
                3 => (8 * 1024 * 1024, u64::MAX),
                7 => (1024, 4096),
                _ => (u64::MAX, u64::MAX),
            };
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&cur.to_le_bytes());
            b[8..].copy_from_slice(&max.to_le_bytes());
            self.uwrite(c, old, &b)?;
        }
        Ok(0)
    }

    fn sys_clock_gettime(&mut self, c: usize, clk: u64, tp: u64) -> Result<i64, i64> {
        let ns = timer::now_ns();
        let total = match clk {
            0 | 5 | 11 => self.lx().epoch * 1_000_000_000 + ns,
            _ => ns,
        };
        let mut ts = [0u8; 16];
        ts[..8].copy_from_slice(&(total / 1_000_000_000).to_le_bytes());
        ts[8..].copy_from_slice(&(total % 1_000_000_000).to_le_bytes());
        self.uwrite(c, tp, &ts).map(|_| 0)
    }
}

/// Write into a (not necessarily active) address space through the kernel
/// direct map. The pages must be mapped.
fn vs_write(vs: &mut AddressSpace, mut va: u64, mut data: &[u8]) {
    while !data.is_empty() {
        let Some(pa) = vs.lookup(va) else { return };
        let n = ((PAGE_SIZE - va % PAGE_SIZE) as usize).min(data.len());
        // SAFETY: a frame owned by `vs`, reached through the direct map.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), phys_to_virt(pa) as *mut u8, n) };
        data = &data[n..];
        va += n as u64;
    }
}
