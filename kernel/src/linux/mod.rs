//! Linux ABI personality.
//!
//! Runs unmodified static Linux arm64 executables (musl, glibc-static,
//! Rust std, busybox) directly on the Flare framekernel: the Linux system
//! call ABI is implemented here on Flare primitives (address spaces,
//! frames, the scheduler) and Flare services (console input and output go
//! through the user-space console driver). There is no Linux kernel
//! underneath.
//!
//! Each Linux thread is a Flare component with a [`Task`]; the threads of a
//! process share one [`Process`] and one address space. Linux tasks hold no
//! Flare capabilities: they can only act through the system calls here.
//!
//! Much of the behaviour (signal frames, the test suite in
//! `compat/linux/tests`) follows QRT's Linux layer
//! (github.com/grinkodbogdan/qrt).

pub mod abi;
pub mod fs;
pub mod mm;
pub mod net;
pub mod signal;
pub mod tty;
pub mod vfs;

use crate::arch::mmu::phys_to_virt;
use crate::arch::timer;
pub(crate) use crate::kernel::Kernel;
use crate::kernel::State;
use crate::mm::vspace::{AddressSpace, Prot};
use crate::mm::{frame, PAGE_SIZE};
use crate::{boot, elf};
use abi::nr;
use abi::*;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{RefCell, RefMut};
use fs::{open_file, Fd, FileKind, FileRef};
pub use mm::USER_MIN_ADDR;
use mm::{Backing, Mm, Vma};
use signal::{ProcSig, SigInfo, ThreadSig};
use tty::Tty;
use vfs::Vfs;

/// Top of the initial stack; the stack may grow down to `STACK_LIMIT`.
pub const STACK_TOP: u64 = 0x7f_ffff_0000;
pub const STACK_LIMIT: u64 = STACK_TOP - 8 * 1024 * 1024;
const STACK_INITIAL: u64 = 128 * 1024;
/// Anonymous mappings are placed downward from here.
pub const MMAP_TOP: u64 = 0x7f_0000_0000;
/// Signal return trampoline (`mov x8, #139; svc #0`), one page per process.
pub const SIGTRAMP: u64 = 0x7f_ffff_e000;
/// Load address of position-independent executables.
const PIE_BASE: u64 = 0x55_5555_0000;
/// Load address of the program interpreter (dynamic linker).
const INTERP_BASE: u64 = 0x7f_4000_0000;
pub const MAX_FDS: usize = 1024;

pub fn proc_files() -> &'static [&'static str] {
    &["version", "uptime", "meminfo", "cpuinfo", "loadavg", "mounts", "filesystems", "cmdline", "stat"]
}

/// A thread group: what the threads of one Linux process share.
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    pub sid: u32,
    pub fds: Vec<Option<Fd>>,
    pub cwd: String,
    pub exe: String,
    pub umask: u32,
    pub mm: Mm,
    pub sig: ProcSig,
    /// Wait status once the process has exited (a zombie until reaped).
    pub status: Option<i32>,
    /// Live threads.
    pub threads: u32,
    pub execed: bool,
}

impl Process {
    pub fn install_fd(&mut self, file: FileRef, cloexec: bool, min: usize) -> Result<usize, i64> {
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

    pub fn file(&self, fd: u64) -> Result<FileRef, i64> {
        self.fds.get(fd as usize).and_then(|f| f.as_ref()).map(|f| f.file.clone()).ok_or(EBADF)
    }
}

/// State of a system call that is waiting (re-run when anything changes).
#[derive(Default, Clone)]
pub struct Wait {
    pub deadline: Option<u64>,
    /// (futex key, woken)
    pub futex: Option<(u64, bool)>,
}

/// One Linux thread.
pub struct Task {
    pub tid: u32,
    pub p: Rc<RefCell<Process>>,
    pub sig: ThreadSig,
    pub clear_tid: u64,
    pub wait: Wait,
    pub vfork_child: Option<u32>,
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
    pub unix_names: BTreeMap<String, Weak<RefCell<net::Socket>>>,
}

pub enum Out {
    Ret(i64),
    /// The task waits; the system call is re-run when something changes.
    Block,
    /// The task exited, or its registers were replaced (exec, sigreturn).
    Gone,
}

pub fn cstr_bytes(v: &[u8]) -> Vec<u8> {
    let mut v = v.to_vec();
    v.push(0);
    v
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

impl Personality {
    pub fn new() -> Personality {
        let mut vfs = Vfs::new();
        match vfs.unpack(boot::INITRAMFS) {
            Ok(n) => kprintln!("linux: initramfs unpacked: {} entries", n),
            Err(e) => kprintln!("linux: initramfs damaged: {}", e),
        }
        Personality {
            vfs,
            tty: Tty::new(),
            next_pid: 1,
            init_finished: false,
            rng: 0x9e37_79b9_7f4a_7c15 ^ timer::now_ns(),
            epoch: rtc_epoch(),
            warned: Vec::new(),
            unix_names: BTreeMap::new(),
        }
    }

    pub fn random(&mut self) -> u64 {
        let mut x = self.rng ^ timer::now_ns();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn epoch_ns(&self) -> u64 {
        self.epoch * 1_000_000_000
    }

    fn alloc_pid(&mut self) -> u32 {
        let p = self.next_pid;
        self.next_pid += 1;
        p
    }

    pub fn proc_content(&self, name: &str, procs: usize) -> Option<Vec<u8>> {
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

impl Kernel {
    pub(crate) fn lx(&mut self) -> &mut Personality {
        self.linux.as_mut().expect("Linux personality not started")
    }

    pub(crate) fn task(&mut self, c: usize) -> &mut Task {
        self.comps[c].linux.as_mut().expect("not a Linux task")
    }

    pub(crate) fn proc(&self, c: usize) -> RefMut<'_, Process> {
        self.comps[c].linux.as_ref().expect("not a Linux task").p.borrow_mut()
    }

    fn prc(&self, c: usize) -> Rc<RefCell<Process>> {
        self.comps[c].linux.as_ref().expect("not a Linux task").p.clone()
    }

    fn live_task(&self, i: usize) -> bool {
        !self.comps[i].free && self.comps[i].state != State::Dead && self.comps[i].linux.is_some()
    }

    /// The component of thread `tid` (a process's leader has tid == pid).
    fn find_tid(&self, tid: u32) -> Option<usize> {
        self.comps.iter().position(|comp| !comp.free && comp.linux.as_ref().is_some_and(|t| t.tid == tid))
    }

    /// Live threads of the process of `c`.
    fn group(&self, c: usize) -> Vec<usize> {
        let p = self.prc(c);
        (0..self.comps.len())
            .filter(|&i| self.live_task(i) && Rc::ptr_eq(&self.comps[i].linux.as_ref().unwrap().p, &p))
            .collect()
    }

    // ---------------------------------------------------------------
    // User memory helpers (populating lazily mapped pages)
    // ---------------------------------------------------------------

    pub(crate) fn uread(&mut self, c: usize, va: u64, len: usize) -> Result<Vec<u8>, i64> {
        if !self.linux_touch(c, va, len, false) {
            return Err(EFAULT);
        }
        self.copy_from_user(c, va, len).map_err(|_| EFAULT)
    }

    pub(crate) fn uwrite(&mut self, c: usize, va: u64, data: &[u8]) -> Result<(), i64> {
        if !self.linux_touch(c, va, data.len(), true) {
            return Err(EFAULT);
        }
        self.copy_to_user(c, va, data).map_err(|_| EFAULT)
    }

    pub(crate) fn uread_u64(&mut self, c: usize, va: u64) -> Result<u64, i64> {
        Ok(u64::from_le_bytes(self.uread(c, va, 8)?.try_into().unwrap()))
    }

    fn uread_u32(&mut self, c: usize, va: u64) -> Result<u32, i64> {
        Ok(u32::from_le_bytes(self.uread(c, va, 4)?.try_into().unwrap()))
    }

    pub(crate) fn ustr(&mut self, c: usize, mut va: u64) -> Result<String, i64> {
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

    fn ustr_array(&mut self, c: usize, mut va: u64) -> Result<Vec<Vec<u8>>, i64> {
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

    fn new_task(&mut self, name: &str, p: Rc<RefCell<Process>>, tid: u32) -> usize {
        let c = self.alloc_comp(name);
        self.comps[c].linux = Some(Box::new(Task {
            tid,
            p,
            sig: ThreadSig::default(),
            clear_tid: 0,
            wait: Wait::default(),
            vfork_child: None,
        }));
        c
    }

    /// Start the Linux personality and its init process (pid 1).
    pub(crate) fn linux_start(&mut self) {
        let Some(init) = boot::LINUX_INIT else { return };
        self.linux = Some(Personality::new());
        let pid = self.lx().alloc_pid();
        let tty = open_file(FileKind::Tty, O_RDWR, "/dev/console");
        let fds = (0..3).map(|_| Some(Fd { file: tty.clone(), cloexec: false })).collect();
        let p = Rc::new(RefCell::new(Process {
            pid,
            ppid: 0,
            pgid: pid,
            sid: pid,
            fds,
            cwd: "/".to_string(),
            exe: String::new(),
            umask: 0o022,
            mm: Mm::default(),
            sig: ProcSig::default(),
            status: None,
            threads: 1,
            execed: false,
        }));
        let c = self.new_task("init", p, pid);
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

    /// Replace the image of task `c`'s process (execve).
    fn linux_exec(&mut self, c: usize, path: &str, mut argv: Vec<Vec<u8>>, envp: Vec<Vec<u8>>) -> Result<(), i64> {
        let cwd = self.proc(c).cwd.clone();
        let mut path = path.to_string();
        let mut depth = 0;
        let image = loop {
            let r = self.lx().vfs.lookup(&cwd, &path, true)?;
            let n = r.node.borrow();
            if !n.is_reg() || n.mode & 0o111 == 0 {
                return Err(EACCES);
            }
            let image = n.data.clone();
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
            break image;
        };

        let asid = self.free_asids.pop().ok_or(ENOMEM)?;
        let Some(mut vs) = AddressSpace::new(asid) else {
            self.free_asids.push(asid);
            return Err(ENOMEM);
        };
        let fail = |k: &mut Kernel, vs: AddressSpace, e: i64| {
            vs.destroy();
            k.free_asids.push(asid);
            Err(e)
        };
        let info = match elf::load(&image, &mut vs, USER_MIN_ADDR..STACK_LIMIT, PIE_BASE) {
            Ok(i) => i,
            Err(e) => {
                kprintln!("linux: exec {}: {}", path, e);
                return fail(self, vs, ENOEXEC);
            }
        };
        // Dynamically linked: the interpreter (e.g. ld-musl, linker64) runs
        // first and loads the program's libraries itself.
        let interp = match &info.interp {
            None => None,
            Some(ipath) => {
                let data = match self.lx().vfs.lookup("/", ipath, true) {
                    Ok(r) if r.node.borrow().is_reg() => r.node.borrow().data.clone(),
                    _ => {
                        kprintln!("linux: exec {}: interpreter {} not found", path, ipath);
                        return fail(self, vs, ENOENT);
                    }
                };
                match elf::load(&data, &mut vs, USER_MIN_ADDR..STACK_LIMIT, INTERP_BASE) {
                    Ok(i) if i.interp.is_none() => Some(i),
                    Ok(_) => return fail(self, vs, ELIBBAD),
                    Err(e) => {
                        kprintln!("linux: exec {}: interpreter {}: {}", path, ipath, e);
                        return fail(self, vs, ELIBBAD);
                    }
                }
            }
        };
        let mut mm = Mm { vmas: Vec::new(), brk_start: info.end, brk: info.end };
        for &(s, e, prot) in info.segments.iter().chain(interp.iter().flat_map(|i| i.segments.iter())) {
            let p = PROT_READ | if prot.write { PROT_WRITE } else { 0 } | if prot.exec { PROT_EXEC } else { 0 };
            mm.carve(s, e);
            mm.insert(Vma { start: s, end: e, prot: p, backing: Backing::Anon });
        }
        mm.insert(Vma { start: STACK_LIMIT, end: STACK_TOP, prot: PROT_READ | PROT_WRITE, backing: Backing::Anon });
        mm.insert(Vma {
            start: SIGTRAMP,
            end: SIGTRAMP + PAGE_SIZE,
            prot: PROT_READ | PROT_EXEC,
            backing: Backing::Anon,
        });
        let mut va = STACK_TOP - STACK_INITIAL;
        while va < STACK_TOP {
            if vs.map_new(va, Prot::RW).is_err() {
                return fail(self, vs, ENOMEM);
            }
            va += PAGE_SIZE;
        }
        match vs.map_new(SIGTRAMP, Prot::RW) {
            Ok(pa) => {
                // mov x8, #139 ; svc #0
                vs_write(&mut vs, SIGTRAMP, &[0x68, 0x11, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4]);
                crate::arch::sync_icache(phys_to_virt(pa), 8);
                vs.protect(SIGTRAMP, Prot { none: false, write: false, exec: true });
            }
            Err(_) => return fail(self, vs, ENOMEM),
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
            (AT_BASE, if interp.is_some() { INTERP_BASE } else { 0 }),
            (AT_FLAGS, 0),
            (AT_ENTRY, info.entry),
            (AT_UID, 0),
            (AT_EUID, 0),
            (AT_GID, 0),
            (AT_EGID, 0),
            (AT_HWCAP, 0x3),
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
        words.extend([AT_SECURE, 0, AT_NULL, 0]);
        let sp = (top - words.len() as u64 * 8) & !15;
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        vs_write(&mut vs, sp, &bytes);

        // Point of no return: other threads go away, the old image is replaced.
        for t in self.group(c) {
            if t != c {
                self.teardown(t);
                self.comps[t].free = true;
                self.comps[t].linux = None;
            }
        }
        if let Some(old) = self.comps[c].vspace.take() {
            if let Ok(old) = Rc::try_unwrap(old) {
                let old = old.into_inner();
                self.free_asids.push(old.asid());
                old.destroy();
            }
        }
        self.comps[c].vspace = Some(Rc::new(RefCell::new(vs)));
        let name = path.rsplit('/').next().unwrap_or("?").to_string();
        let comp = &mut self.comps[c];
        comp.name = name;
        comp.frame = crate::arch::TrapFrame::zeroed();
        comp.frame.elr = interp.as_ref().map_or(info.entry, |i| i.entry);
        comp.frame.sp_el0 = sp;
        *comp.ext = Default::default();
        if self.ext_owner == Some(c) {
            self.ext_owner = None;
        }
        // A thread that execs takes over the leader's tid (Linux semantics).
        let pid = self.proc(c).pid;
        let t = self.task(c);
        t.tid = pid;
        t.sig.alt_size = 0;
        let mut p = self.proc(c);
        p.mm = mm;
        p.exe = vfs::absolute(&cwd, &path);
        p.execed = true;
        p.threads = 1;
        for fd in p.fds.iter_mut() {
            if fd.as_ref().is_some_and(|f| f.cloexec) {
                *fd = None;
            }
        }
        for a in p.sig.actions.iter_mut() {
            if a.handler != SIG_IGN {
                *a = signal::SigAction::default();
            }
        }
        Ok(())
    }

    /// One thread ends. The process ends with its last thread.
    fn linux_exit_thread(&mut self, c: usize, status: i32) {
        let clear = self.task(c).clear_tid;
        if clear != 0 {
            let _ = self.uwrite(c, clear, &0u32.to_le_bytes());
            if let Some(key) = self.futex_key(c, clear) {
                self.futex_wake(key, 1, None);
            }
        }
        let remaining = {
            let mut p = self.proc(c);
            p.threads -= 1;
            p.threads
        };
        if remaining == 0 {
            self.linux_process_dead(c, status);
            return;
        }
        let is_leader = {
            let t = self.comps[c].linux.as_ref().unwrap();
            t.tid == t.p.borrow().pid
        };
        self.teardown(c);
        if !is_leader {
            self.comps[c].free = true;
            self.comps[c].linux = None;
        }
    }

    /// The whole process ends (exit_group, a fatal signal).
    pub(crate) fn linux_exit_group(&mut self, c: usize, status: i32) {
        if self.comps[c].linux.is_none() || self.proc(c).status.is_some() {
            return;
        }
        let pid = self.proc(c).pid;
        for t in self.group(c) {
            if t != c && self.comps[t].linux.as_ref().unwrap().tid != pid {
                self.teardown(t);
                self.comps[t].free = true;
                self.comps[t].linux = None;
            }
        }
        self.linux_process_dead(c, status);
    }

    fn linux_process_dead(&mut self, c: usize, status: i32) {
        let p = self.prc(c);
        let (pid, ppid) = {
            let mut pb = p.borrow_mut();
            pb.status = Some(status);
            pb.threads = 0;
            pb.fds.clear();
            (pb.pid, pb.ppid)
        };
        // Reparent children to init.
        for comp in self.comps.iter_mut() {
            if let Some(t) = comp.linux.as_ref() {
                if !comp.free && !Rc::ptr_eq(&t.p, &p) {
                    if let Ok(mut q) = t.p.try_borrow_mut() {
                        if q.ppid == pid {
                            q.ppid = 1;
                        }
                    }
                }
            }
        }
        // The zombie lives on in the leader's component (or this one).
        let leader = self.find_tid(pid).unwrap_or(c);
        for t in [c, leader] {
            if self.comps[t].linux.is_some() && self.comps[t].state != State::Dead {
                self.teardown(t);
            }
        }
        if c != leader {
            self.comps[c].free = true;
            self.comps[c].linux = None;
        }
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
        let parent = if ppid == 0 { None } else { self.find_tid(ppid).filter(|&i| self.live_task(i)) };
        match parent {
            Some(pc) => {
                let code = if status & 0x7f == 0 {
                    signal::CLD_EXITED
                } else if status & 0x80 != 0 {
                    signal::CLD_DUMPED
                } else {
                    signal::CLD_KILLED
                };
                let st = if status & 0x7f == 0 { (status >> 8) & 0xff } else { status & 0x7f };
                let info = SigInfo { signo: SIGCHLD as i32, code, pid, status: st, ..Default::default() };
                self.sig_send_proc(pc, info);
            }
            None => {
                self.comps[leader].free = true;
                self.comps[leader].linux = None;
            }
        }
    }

    fn linux_kill(&mut self, c: usize, pid: i64, sig: u64) -> i64 {
        if sig > 64 {
            return -EINVAL;
        }
        let (me, my_pgid) = {
            let p = self.proc(c);
            (p.pid, p.pgid)
        };
        let targets: Vec<usize> = (0..self.comps.len())
            .filter(|&i| self.live_task(i))
            .filter(|&i| {
                let t = self.comps[i].linux.as_ref().unwrap();
                let p = t.p.borrow();
                t.tid == p.pid
                    && match pid {
                        p0 if p0 > 0 => p.pid == p0 as u32,
                        0 => p.pgid == my_pgid,
                        -1 => p.pid != 1 && p.pid != me,
                        g => p.pgid == (-g) as u32,
                    }
            })
            .collect();
        if targets.is_empty() {
            return -ESRCH;
        }
        for t in targets {
            if self.live_task(t) {
                self.sig_send_proc(t, SigInfo::user(sig, me));
            }
        }
        0
    }

    /// Feed console input to the TTY (from the console driver).
    pub(crate) fn linux_tty_input(&mut self, bytes: &[u8]) {
        let Some(lx) = self.linux.as_mut() else { return };
        lx.tty.input(bytes);
        if core::mem::take(&mut lx.tty.interrupt) {
            let pg = lx.tty.fg_pgrp;
            let targets: Vec<usize> = (0..self.comps.len())
                .filter(|&i| self.live_task(i))
                .filter(|&i| {
                    let t = self.comps[i].linux.as_ref().unwrap();
                    let p = t.p.borrow();
                    t.tid == p.pid && p.pgid == pg && p.pid != 1
                })
                .collect();
            for t in targets {
                if self.live_task(t) {
                    self.sig_send_proc(
                        t,
                        SigInfo { signo: SIGINT as i32, code: signal::SI_KERNEL, ..Default::default() },
                    );
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Futexes
    // ---------------------------------------------------------------

    /// Physical address of a futex word: the same for every mapping.
    fn futex_key(&mut self, c: usize, uaddr: u64) -> Option<u64> {
        if !uaddr.is_multiple_of(4) || !self.linux_touch(c, uaddr, 4, false) {
            return None;
        }
        self.comps[c].vspace.as_ref()?.borrow().lookup(uaddr)
    }

    /// Wake up to `n` waiters on `key` (moving up to `requeue.1` more to `requeue.0`).
    fn futex_wake(&mut self, key: u64, n: u64, requeue: Option<(u64, u64)>) -> i64 {
        let mut woken = 0u64;
        let mut moved = 0u64;
        for i in 0..self.comps.len() {
            if !self.live_task(i) || self.comps[i].state != State::LinuxWait {
                continue;
            }
            let t = self.comps[i].linux.as_mut().unwrap();
            if let Some((k, w)) = t.wait.futex.as_mut() {
                if *k == key && !*w {
                    if woken < n {
                        *w = true;
                        woken += 1;
                    } else if let Some((nk, max)) = requeue {
                        if moved < max {
                            *k = nk;
                            moved += 1;
                        }
                    }
                }
            }
        }
        woken as i64
    }

    fn sys_futex(&mut self, c: usize, a: [u64; 6]) -> Out {
        let (uaddr, op, val, tmo, uaddr2, val3) = (a[0], a[1] & 0x7f, a[2], a[3], a[4], a[5]);
        // A waiter that is re-run: woken, timed out, or still waiting.
        if let Some((_, woken)) = self.task(c).wait.futex {
            if woken {
                self.task(c).wait = Wait::default();
                return Out::Ret(0);
            }
            if let Some(d) = self.task(c).wait.deadline {
                if timer::now_ns() >= d {
                    self.task(c).wait = Wait::default();
                    return Out::Ret(-ETIMEDOUT);
                }
            }
            return Out::Block;
        }
        let Some(key) = self.futex_key(c, uaddr) else { return Out::Ret(-EFAULT) };
        match op {
            0 | 9 => {
                // FUTEX_WAIT / FUTEX_WAIT_BITSET
                let cur = match self.uread_u32(c, uaddr) {
                    Ok(v) => v,
                    Err(e) => return Out::Ret(-e),
                };
                if cur != val as u32 {
                    return Out::Ret(-EAGAIN);
                }
                let deadline = if tmo != 0 {
                    let (s, ns) = match (self.uread_u64(c, tmo), self.uread_u64(c, tmo + 8)) {
                        (Ok(s), Ok(ns)) => (s, ns),
                        _ => return Out::Ret(-EFAULT),
                    };
                    let t = s * 1_000_000_000 + ns;
                    Some(if op == 9 {
                        // Absolute; CLOCK_REALTIME if FUTEX_CLOCK_REALTIME.
                        if a[1] & 256 != 0 {
                            t.saturating_sub(self.lx().epoch_ns())
                        } else {
                            t
                        }
                    } else {
                        timer::now_ns() + t
                    })
                } else {
                    None
                };
                let w = &mut self.task(c).wait;
                w.futex = Some((key, false));
                w.deadline = deadline;
                Out::Block
            }
            1 | 10 => Out::Ret(self.futex_wake(key, val, None)),
            3 | 4 => {
                // FUTEX_REQUEUE / FUTEX_CMP_REQUEUE
                if op == 4 {
                    match self.uread_u32(c, uaddr) {
                        Ok(v) if v == val3 as u32 => {}
                        Ok(_) => return Out::Ret(-EAGAIN),
                        Err(e) => return Out::Ret(-e),
                    }
                }
                let Some(key2) = self.futex_key(c, uaddr2) else { return Out::Ret(-EFAULT) };
                Out::Ret(self.futex_wake(key, val, Some((key2, tmo))))
            }
            5 => {
                // FUTEX_WAKE_OP: wake on both words (the operation is not applied).
                let n = self.futex_wake(key, val, None);
                let m = match self.futex_key(c, uaddr2) {
                    Some(k2) => self.futex_wake(k2, tmo, None),
                    None => 0,
                };
                Out::Ret(n + m)
            }
            _ => Out::Ret(-ENOSYS),
        }
    }

    // ---------------------------------------------------------------
    // Entry points
    // ---------------------------------------------------------------

    pub(crate) fn linux_syscall(&mut self, c: usize) {
        match self.linux_dispatch(c) {
            Out::Ret(v) => self.comps[c].frame.x[0] = v as u64,
            Out::Block => self.comps[c].state = State::LinuxWait,
            Out::Gone => {}
        }
    }

    /// Re-run the system calls of waiting tasks; deliver timers.
    pub(crate) fn linux_poll(&mut self) {
        if self.linux.is_none() {
            return;
        }
        self.linux_timers();
        for c in 0..self.comps.len() {
            if self.comps[c].state != State::LinuxWait || self.comps[c].free {
                continue;
            }
            if self.linux_interrupt(c) {
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
                let old = {
                    let mut p = self.proc(c);
                    match p.fds.get_mut(a[0] as usize) {
                        Some(slot @ Some(_)) => slot.take(),
                        _ => None,
                    }
                };
                match old {
                    Some(f) => {
                        drop(f);
                        Ok(0)
                    }
                    None => Err(EBADF),
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
                let mut p = self.proc(c);
                p.file(a[0]).and_then(|f| p.install_fd(f, false, 0)).map(|n| n as i64)
            }
            nr::DUP3 => self.sys_dup3(c, a[0], a[1], a[2] as u32),
            nr::FCNTL => self.sys_fcntl(c, a[0], a[1], a[2]),
            nr::IOCTL => self.sys_ioctl(c, a[0], a[1], a[2]),
            nr::PIPE2 => self.sys_pipe2(c, a[0], a[1] as u32),
            nr::PPOLL => return self.sys_ppoll(c, a[0], a[1], a[2]),
            nr::PSELECT6 => Err(ENOSYS),
            nr::SENDFILE => self.sys_sendfile(c, a[0], a[1], a[2], a[3]),
            nr::SYNC | nr::FSYNC | nr::FADVISE64 | nr::MSYNC | nr::MLOCK => Ok(0),
            nr::EVENTFD2 => self.sys_eventfd2(c, a[0], a[1]),
            nr::EPOLL_CREATE1 => self.sys_epoll_create1(c, a[0]),
            nr::EPOLL_CTL => self.sys_epoll_ctl(c, a[0], a[1], a[2], a[3]),
            nr::EPOLL_PWAIT => return self.sys_epoll_pwait(c, a[0], a[1], a[2], a[3] as i32 as i64),
            nr::EPOLL_PWAIT2 => {
                let ms = if a[3] == 0 {
                    -1
                } else {
                    match (self.uread_u64(c, a[3]), self.uread_u64(c, a[3] + 8)) {
                        (Ok(s), Ok(ns)) => (s * 1000 + ns / 1_000_000) as i64,
                        _ => return Out::Ret(-EFAULT),
                    }
                };
                return self.sys_epoll_pwait(c, a[0], a[1], a[2], ms);
            }
            nr::TIMERFD_CREATE => self.sys_timerfd_create(c, a[0], a[1]),
            nr::TIMERFD_SETTIME => self.sys_timerfd_settime(c, a[0], a[1], a[2], a[3]),
            nr::TIMERFD_GETTIME => self.sys_timerfd_gettime(c, a[0], a[1]),
            nr::SIGNALFD4 => self.sys_signalfd4(c, a[0], a[1], a[3]),
            nr::MEMFD_CREATE => self.sys_memfd_create(c, a[0], a[1]),

            nr::SOCKET => self.sys_socket(c, a[0], a[1], a[2]),
            nr::SOCKETPAIR => self.sys_socketpair(c, a[0], a[1], a[2], a[3]),
            nr::BIND => self.sys_bind(c, a[0], a[1], a[2]),
            nr::LISTEN => self.sys_listen(c, a[0], a[1]),
            nr::CONNECT => self.sys_connect(c, a[0], a[1], a[2]),
            nr::ACCEPT => return self.sys_accept4(c, a[0], a[1], a[2], 0),
            nr::ACCEPT4 => return self.sys_accept4(c, a[0], a[1], a[2], a[3]),
            nr::GETSOCKNAME => self.sys_getsockname(c, a[0], a[1], a[2], false),
            nr::GETPEERNAME => self.sys_getsockname(c, a[0], a[1], a[2], true),
            nr::GETSOCKOPT => self.sys_getsockopt(c, a[0], a[1], a[2], a[3], a[4]),
            nr::SETSOCKOPT => Ok(0),
            nr::SHUTDOWN => self.sys_shutdown(c, a[0], a[1]),
            nr::SENDTO => self.sys_sendto(c, a[0], a[1], a[2], a[3], a[4], a[5]),
            nr::RECVFROM => return self.sys_recvfrom(c, a[0], a[1], a[2], a[3], a[4], a[5]),
            nr::SENDMSG => self.sys_sendmsg(c, a[0], a[1], a[2]),
            nr::RECVMSG => return self.sys_recvmsg(c, a[0], a[1], a[2]),

            nr::BRK => Ok(self.sys_brk(c, a[0]) as i64),
            nr::MMAP => return self.mmap_dispatch(c, a),
            nr::MUNMAP => self.sys_munmap(c, a[0], a[1]),
            nr::MPROTECT => self.sys_mprotect(c, a[0], a[1], a[2]),
            nr::MREMAP => self.sys_mremap(c, a[0], a[1], a[2], a[3], a[4]),
            nr::MADVISE => self.sys_madvise(c, a[0], a[1], a[2]),

            nr::CLONE => return self.sys_clone(c, a[0], a[1], a[2], a[3], a[4]),
            nr::CLONE3 => Err(ENOSYS),
            nr::EXECVE => return self.sys_execve(c, a[0], a[1], a[2]),
            nr::EXIT => {
                self.linux_exit_thread(c, ((a[0] & 0xff) << 8) as i32);
                return Out::Gone;
            }
            nr::EXIT_GROUP => {
                self.linux_exit_group(c, ((a[0] & 0xff) << 8) as i32);
                return Out::Gone;
            }
            nr::WAIT4 => return self.sys_wait4(c, a[0] as i32 as i64, a[1], a[2]),
            nr::KILL => Ok(self.linux_kill(c, a[0] as i32 as i64, a[1])),
            nr::TKILL | nr::TGKILL => {
                let (tid, sig) = if f.x[8] == nr::TGKILL { (a[1], a[2]) } else { (a[0], a[1]) };
                match self.find_tid(tid as u32).filter(|&i| self.live_task(i)) {
                    Some(t) => {
                        if sig != 0 {
                            let me = self.proc(c).pid;
                            let info =
                                SigInfo { signo: sig as i32, code: signal::SI_TKILL, pid: me, ..Default::default() };
                            self.sig_send_thread(t, info);
                        }
                        Ok(0)
                    }
                    None => Err(ESRCH),
                }
            }
            nr::RT_SIGQUEUEINFO => Ok(self.linux_kill(c, a[0] as i32 as i64, a[1])),
            nr::RT_SIGACTION => self.sys_sigaction(c, a[0], a[1], a[2]),
            nr::RT_SIGPROCMASK => self.sys_sigprocmask(c, a[0], a[1], a[2]),
            nr::RT_SIGPENDING => self.sys_sigpending(c, a[0]),
            nr::RT_SIGSUSPEND => return self.sys_sigsuspend(c, a[0]),
            nr::RT_SIGTIMEDWAIT => return self.sys_sigtimedwait(c, a[0], a[1], a[2]),
            nr::RT_SIGRETURN => return self.sys_rt_sigreturn(c),
            nr::SIGALTSTACK => self.sys_sigaltstack(c, a[0], a[1]),
            nr::SETITIMER => self.sys_setitimer(c, a[0], a[1], a[2]),
            nr::GETITIMER => self.sys_setitimer(c, a[0], 0, a[1]),
            nr::NANOSLEEP | nr::CLOCK_NANOSLEEP => {
                let req = if f.x[8] == nr::NANOSLEEP { a[0] } else { a[2] };
                return self.sys_nanosleep(c, req, f.x[8] == nr::CLOCK_NANOSLEEP && a[1] & 1 != 0);
            }
            nr::SCHED_YIELD => {
                self.need_resched = true;
                Ok(0)
            }
            nr::FUTEX => return self.sys_futex(c, a),
            nr::GETPID => Ok(self.proc(c).pid as i64),
            nr::GETTID => Ok(self.task(c).tid as i64),
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
                match self.find_tid(pid) {
                    Some(i) => Ok(self.proc(i).pgid as i64),
                    None => Err(ESRCH),
                }
            }
            nr::GETSID => Ok(self.proc(c).sid as i64),
            nr::SETSID => {
                let mut p = self.proc(c);
                p.sid = p.pid;
                p.pgid = p.pid;
                Ok(p.pid as i64)
            }
            nr::SET_TID_ADDRESS => {
                let t = self.task(c);
                t.clear_tid = a[0];
                Ok(t.tid as i64)
            }
            nr::SET_ROBUST_LIST | nr::PRCTL | nr::SETPRIORITY => Ok(0),
            nr::GETPRIORITY => Ok(20),
            nr::RSEQ | nr::MEMBARRIER => Err(ENOSYS),
            nr::UMASK => {
                let mut p = self.proc(c);
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
                if a[0] != 0 {
                    let _ = self.uwrite(c, a[0], &[0u8; 32]);
                }
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
            n => {
                self.warn_unimplemented(n);
                Err(ENOSYS)
            }
        };
        match r {
            Ok(v) => Out::Ret(v),
            Err(e) => Out::Ret(-e),
        }
    }

    // ----- processes -----

    fn sys_clone(&mut self, c: usize, flags: u64, stack: u64, ptid: u64, tls: u64, ctid: u64) -> Out {
        // A vfork parent resumes once the child has exec'd or exited.
        if let Some(child) = self.task(c).vfork_child {
            let done = match self.find_tid(child) {
                None => true,
                Some(i) => self.comps[i].state == State::Dead || self.proc(i).execed,
            };
            if !done {
                return Out::Block;
            }
            self.task(c).vfork_child = None;
            return Out::Ret(child as i64);
        }
        self.save_ext(c);
        let tid = self.lx().alloc_pid();
        let name = self.comps[c].name.clone();
        let (n, vs) = if flags & CLONE_THREAD != 0 {
            let p = self.prc(c);
            p.borrow_mut().threads += 1;
            (self.new_task(&name, p, tid), self.comps[c].vspace.clone().unwrap())
        } else {
            let Some(asid) = self.free_asids.pop() else { return Out::Ret(-EAGAIN) };
            let dup = self.comps[c].vspace.as_ref().unwrap().borrow().duplicate(asid);
            let Some(vs) = dup else {
                self.free_asids.push(asid);
                return Out::Ret(-ENOMEM);
            };
            let child = {
                let p = self.proc(c);
                Process {
                    pid: tid,
                    ppid: p.pid,
                    pgid: p.pgid,
                    sid: p.sid,
                    fds: p.fds.clone(),
                    cwd: p.cwd.clone(),
                    exe: p.exe.clone(),
                    umask: p.umask,
                    mm: p.mm.clone(),
                    sig: ProcSig { pending: 0, itimer: None, ..p.sig.clone() },
                    status: None,
                    threads: 1,
                    execed: false,
                }
            };
            (self.new_task(&name, Rc::new(RefCell::new(child)), tid), Rc::new(RefCell::new(vs)))
        };
        let mut frame = self.comps[c].frame;
        frame.x[0] = 0;
        if stack != 0 {
            frame.sp_el0 = stack;
        }
        let mut ext = self.comps[c].ext.clone();
        if flags & CLONE_SETTLS != 0 {
            ext.tpidr = tls;
        }
        let mask = self.task(c).sig.mask;
        {
            let ch = &mut self.comps[n];
            ch.vspace = Some(vs);
            ch.frame = frame;
            ch.ext = ext;
            ch.linux.as_mut().unwrap().sig.mask = mask;
        }
        if flags & CLONE_CHILD_CLEARTID != 0 {
            self.task(n).clear_tid = ctid;
        }
        if flags & CLONE_CHILD_SETTID != 0 {
            let _ = self.uwrite(n, ctid, &tid.to_le_bytes());
        }
        if flags & CLONE_PARENT_SETTID != 0 {
            let _ = self.uwrite(c, ptid, &tid.to_le_bytes());
        }
        self.make_ready(n);
        if flags & CLONE_VFORK != 0 {
            self.task(c).vfork_child = Some(tid);
            return Out::Block;
        }
        Out::Ret(tid as i64)
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
            Err(e) => Out::Ret(-e),
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
            let Some(t) = comp.linux.as_ref() else { continue };
            if comp.free {
                continue;
            }
            let Ok(p) = t.p.try_borrow() else { continue };
            if p.ppid != me || t.tid != p.pid {
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
                        return Out::Ret(-e);
                    }
                }
                self.comps[i].free = true;
                self.comps[i].linux = None;
                Out::Ret(cpid as i64)
            }
            None if !has_child => Out::Ret(-ECHILD),
            None if options & WNOHANG != 0 => Out::Ret(0),
            None => Out::Block,
        }
    }

    fn sys_setpgid(&mut self, c: usize, pid: u32, pgid: u32) -> Result<i64, i64> {
        let me = self.proc(c).pid;
        let target = if pid == 0 { c } else { self.find_tid(pid).ok_or(ESRCH)? };
        let mut p = self.proc(target);
        if p.pid != me && p.ppid != me {
            return Err(ESRCH);
        }
        p.pgid = if pgid == 0 { p.pid } else { pgid };
        Ok(0)
    }

    fn sys_nanosleep(&mut self, c: usize, req: u64, absolute: bool) -> Out {
        let now = timer::now_ns();
        let deadline = match self.task(c).wait.deadline {
            Some(d) => d,
            None => {
                let (s, ns) = match (self.uread_u64(c, req), self.uread_u64(c, req + 8)) {
                    (Ok(s), Ok(ns)) => (s, ns),
                    _ => return Out::Ret(-EFAULT),
                };
                let t = s.saturating_mul(1_000_000_000).saturating_add(ns);
                let d = if absolute { t } else { now.saturating_add(t) };
                self.task(c).wait.deadline = Some(d);
                d
            }
        };
        if now >= deadline {
            self.task(c).wait = Wait::default();
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
                7 => (MAX_FDS as u64, MAX_FDS as u64),
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
            0 | 5 | 11 => self.lx().epoch_ns() + ns,
            _ => ns,
        };
        let mut ts = [0u8; 16];
        ts[..8].copy_from_slice(&(total / 1_000_000_000).to_le_bytes());
        ts[8..].copy_from_slice(&(total % 1_000_000_000).to_le_bytes());
        self.uwrite(c, tp, &ts).map(|_| 0)
    }
}
