//! Open files of the Linux personality: regular files and devices, pipes,
//! eventfd, timerfd, signalfd, epoll and memfd, and the file system calls.

use super::abi::*;
use super::mm::{MemObj, MemRef};
use super::net::Socket;
use super::vfs::{self, NodeRef};
use super::{Kernel, Out, MAX_FDS};
use crate::arch::timer;
use alloc::collections::VecDeque;
use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

pub struct Pipe {
    pub buf: VecDeque<u8>,
    pub readers: usize,
    pub writers: usize,
}

pub struct EventFd {
    count: u64,
    semaphore: bool,
}

pub struct TimerFd {
    realtime: bool,
    /// Next expiry (monotonic ns) and period.
    deadline: Option<u64>,
    interval: u64,
    ticks: u64,
}

impl TimerFd {
    fn update(&mut self, now: u64) {
        if let Some(d) = self.deadline {
            if now >= d {
                let n = (now - d).checked_div(self.interval).map_or(1, |k| k + 1);
                self.ticks += n;
                self.deadline = if self.interval > 0 { Some(d + n * self.interval) } else { None };
            }
        }
    }
}

pub struct EpollEntry {
    fd: i32,
    file: Weak<RefCell<OpenFile>>,
    events: u32,
    data: u64,
    disabled: bool,
}

pub struct Epoll {
    entries: Vec<EpollEntry>,
}

pub enum FileKind {
    Tty,
    Null,
    Zero,
    Random,
    Kmsg,
    Reg(NodeRef),
    Dir(NodeRef),
    PipeR(Rc<RefCell<Pipe>>),
    PipeW(Rc<RefCell<Pipe>>),
    /// Generated content (`/proc` files).
    Mem(Vec<u8>),
    MemFd(MemRef),
    EventFd(Rc<RefCell<EventFd>>),
    TimerFd(Rc<RefCell<TimerFd>>),
    SignalFd(u64),
    Epoll(Rc<RefCell<Epoll>>),
    Socket(Rc<RefCell<Socket>>),
    Binder(Rc<RefCell<super::binder::BinderProc>>),
    /// The supervisor's control device: write a command, read the answer.
    FlareCtl,
    /// /dev/dri/card0
    Drm(super::drm::DrmFileRef),
    /// /dev/input/eventN
    Evdev(super::input::EvdevRef),
    /// A serial device (`boot::SERIAL_DEVICES`): raw bytes to its driver.
    Serial(usize),
    /// /dev/fb0: the display's pixels.
    Fb,
    /// /dev/flarehw: hardware for a Linux driver host.
    FlareHw,
}

pub struct OpenFile {
    pub kind: FileKind,
    pub pos: u64,
    pub flags: u32,
    pub path: String,
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        match &self.kind {
            FileKind::PipeR(p) => p.borrow_mut().readers -= 1,
            FileKind::PipeW(p) => p.borrow_mut().writers -= 1,
            FileKind::Socket(s) => s.borrow_mut().close(),
            _ => {}
        }
    }
}

pub type FileRef = Rc<RefCell<OpenFile>>;

#[derive(Clone)]
pub struct Fd {
    pub file: FileRef,
    pub cloexec: bool,
}

pub fn open_file(kind: FileKind, flags: u32, path: &str) -> FileRef {
    Rc::new(RefCell::new(OpenFile { kind, pos: 0, flags, path: path.to_string() }))
}

pub const EPOLLIN: u32 = 0x1;
pub const EPOLLOUT: u32 = 0x4;
pub const EPOLLERR: u32 = 0x8;
pub const EPOLLHUP: u32 = 0x10;
pub const EPOLLRDHUP: u32 = 0x2000;
pub const EPOLLONESHOT: u32 = 1 << 30;

fn stat_bytes(ino: u64, mode: u32, size: u64, rdev: u64, mtime: u64) -> [u8; 128] {
    let mut b = [0u8; 128];
    let mut put = |off: usize, v: &[u8]| b[off..off + v.len()].copy_from_slice(v);
    put(0, &1u64.to_le_bytes());
    put(8, &ino.to_le_bytes());
    put(16, &mode.to_le_bytes());
    put(20, &1u32.to_le_bytes());
    put(32, &rdev.to_le_bytes());
    put(48, &size.to_le_bytes());
    put(56, &4096u32.to_le_bytes());
    put(64, &size.div_ceil(512).to_le_bytes());
    for off in [72, 88, 104] {
        put(off, &mtime.to_le_bytes());
    }
    b
}

pub fn node_stat(n: &NodeRef) -> [u8; 128] {
    let n = n.borrow();
    let size = if n.is_dir() { 4096 } else { n.data.len() as u64 };
    stat_bytes(n.ino, n.mode, size, n.rdev, n.mtime)
}

fn read_u64_le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

impl Kernel {
    /// Readiness of a file (poll events) as seen by task `c`.
    pub(crate) fn file_poll(&self, c: usize, f: &FileRef) -> u32 {
        let f = f.borrow();
        match &f.kind {
            FileKind::Tty => {
                let readable = self.linux.as_ref().unwrap().tty.readable();
                (if readable { EPOLLIN } else { 0 }) | EPOLLOUT
            }
            FileKind::PipeR(p) => {
                let p = p.borrow();
                (if !p.buf.is_empty() { EPOLLIN } else { 0 }) | (if p.writers == 0 { EPOLLHUP } else { 0 })
            }
            FileKind::PipeW(p) => {
                if p.borrow().readers == 0 {
                    EPOLLOUT | EPOLLERR
                } else {
                    EPOLLOUT
                }
            }
            FileKind::EventFd(e) => (if e.borrow().count > 0 { EPOLLIN } else { 0 }) | EPOLLOUT,
            FileKind::TimerFd(t) => {
                let mut t = t.borrow_mut();
                t.update(timer::now_ns());
                if t.ticks > 0 {
                    EPOLLIN
                } else {
                    0
                }
            }
            FileKind::SignalFd(mask) => {
                let t = self.comps[c].linux.as_ref().unwrap();
                if (t.sig.pending | t.p.borrow().sig.pending) & mask != 0 {
                    EPOLLIN
                } else {
                    0
                }
            }
            FileKind::Socket(s) => s.borrow().poll(),
            FileKind::Binder(b) => {
                let tid = self.comps[c].linux.as_ref().unwrap().tid;
                (if b.borrow().readable(tid) { EPOLLIN } else { 0 }) | EPOLLOUT
            }
            FileKind::Epoll(_) => 0,
            FileKind::Drm(d) => self.drm_poll(d),
            FileKind::Evdev(e) => self.evdev_poll(e),
            FileKind::FlareHw => {
                if self.flarehw_readable() {
                    EPOLLIN
                } else {
                    0
                }
            }
            FileKind::Serial(d) => {
                let p = &self.serials[*d];
                (if p.rx.is_empty() { 0 } else { EPOLLIN })
                    | (if p.tx.len() < crate::kernel::SERIAL_BUF { EPOLLOUT } else { 0 })
            }
            _ => EPOLLIN | EPOLLOUT,
        }
    }

    /// Read from a file. `Ok(None)` means "would block".
    pub(crate) fn file_read(&mut self, c: usize, file: &FileRef, max: usize) -> Result<Option<Vec<u8>>, i64> {
        let mut f = file.borrow_mut();
        let pos = f.pos as usize;
        match &f.kind {
            FileKind::Tty => {
                drop(f);
                Ok(self.lx().tty.read(max))
            }
            FileKind::Null | FileKind::Kmsg => Ok(Some(Vec::new())),
            FileKind::Zero => Ok(Some(vec![0; max.min(65536)])),
            FileKind::Random => {
                drop(f);
                let n = max.min(4096);
                let lx = self.lx();
                Ok(Some((0..n).map(|_| lx.random() as u8).collect()))
            }
            FileKind::Reg(n) => {
                let out = n.borrow().data.read(pos, max);
                f.pos += out.len() as u64;
                Ok(Some(out))
            }
            FileKind::Mem(d) => {
                let end = (pos + max).min(d.len());
                let out = if pos < end { d[pos..end].to_vec() } else { Vec::new() };
                f.pos += out.len() as u64;
                Ok(Some(out))
            }
            FileKind::MemFd(o) => {
                let out = o.borrow_mut().read(pos as u64, max);
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
            FileKind::EventFd(e) => {
                if max < 8 {
                    return Err(EINVAL);
                }
                let mut e = e.borrow_mut();
                if e.count == 0 {
                    return Ok(None);
                }
                let v = if e.semaphore { 1 } else { e.count };
                e.count -= v;
                Ok(Some(v.to_le_bytes().to_vec()))
            }
            FileKind::TimerFd(t) => {
                if max < 8 {
                    return Err(EINVAL);
                }
                let mut t = t.borrow_mut();
                t.update(timer::now_ns());
                if t.ticks == 0 {
                    return Ok(None);
                }
                let v = core::mem::take(&mut t.ticks);
                Ok(Some(v.to_le_bytes().to_vec()))
            }
            FileKind::SignalFd(mask) => {
                let mask = *mask;
                drop(f);
                let mut out = Vec::new();
                while out.len() + 128 <= max {
                    match self.sig_take(c, mask) {
                        Some(si) => out.extend_from_slice(&si.to_signalfd()),
                        None => break,
                    }
                }
                if out.is_empty() {
                    if max < 128 {
                        return Err(EINVAL);
                    }
                    return Ok(None);
                }
                Ok(Some(out))
            }
            FileKind::Socket(s) => {
                let s = s.clone();
                drop(f);
                let mut s = s.borrow_mut();
                s.recv_bytes(max)
            }
            FileKind::PipeW(_) | FileKind::Epoll(_) => Err(EBADF),
            FileKind::Binder(_) => Err(EINVAL),
            FileKind::Drm(d) => {
                let d = d.clone();
                drop(f);
                Ok(self.drm_read(&d, max))
            }
            FileKind::Evdev(e) => {
                let e = e.clone();
                drop(f);
                self.evdev_read(&e, max)
            }
            FileKind::FlareHw => {
                drop(f);
                self.flarehw_read(max)
            }
            FileKind::Serial(d) => {
                let d = *d;
                drop(f);
                let rx = &mut self.serials[d].rx;
                if rx.is_empty() {
                    return Ok(None);
                }
                let n = max.min(rx.len());
                Ok(Some(rx.drain(..n).collect()))
            }
            FileKind::Fb => {
                drop(f);
                let data = self.fb_read(pos as u64, max);
                file.borrow_mut().pos += data.len() as u64;
                Ok(Some(data))
            }
            FileKind::FlareCtl => {
                drop(f);
                let resp = &mut self.lx().ctl_response;
                let n = max.min(resp.len());
                Ok(Some(resp.drain(..n).collect()))
            }
        }
    }

    pub(crate) fn file_write(&mut self, c: usize, file: &FileRef, data: &[u8]) -> Result<usize, i64> {
        let mut f = file.borrow_mut();
        let append = f.flags & O_APPEND != 0;
        if let FileKind::FlareCtl = f.kind {
            drop(f);
            let cmd = String::from_utf8_lossy(data).into_owned();
            let pid = self.proc(c).pid;
            kprintln!("linux: pid {} asks the supervisor: {}", pid, cmd.trim());
            let resp = self.supervisor(cmd.trim()).into_bytes();
            self.lx().ctl_response = resp;
            return Ok(data.len());
        }
        let r = match &f.kind {
            FileKind::Tty => {
                drop(f);
                self.lx().tty.write(data);
                return Ok(data.len());
            }
            FileKind::Serial(d) => {
                let d = *d;
                drop(f);
                let room = crate::kernel::SERIAL_BUF.saturating_sub(self.serials[d].tx.len());
                if room == 0 {
                    return Err(EAGAIN);
                }
                let n = data.len().min(room);
                self.serials[d].tx.extend(data[..n].iter());
                self.serial_kick(d);
                return Ok(n);
            }
            FileKind::Fb => {
                let pos = f.pos;
                drop(f);
                let n = self.fb_write(pos, data);
                file.borrow_mut().pos += n as u64;
                return Ok(n);
            }
            FileKind::Kmsg => {
                crate::klog::write(data);
                return Ok(data.len());
            }
            FileKind::Null | FileKind::Zero | FileKind::Random => Ok(data.len()),
            FileKind::Reg(n) => {
                if f.flags & O_ACCMODE == 0 {
                    return Err(EBADF);
                }
                let n = n.clone();
                let mut node = n.borrow_mut();
                let pos = if append { node.data.len() } else { f.pos as usize };
                let done = node.data.write(pos, data);
                f.pos = (pos + done) as u64;
                if done == 0 && !data.is_empty() {
                    Err(super::abi::ENOSPC)
                } else {
                    Ok(done)
                }
            }
            FileKind::MemFd(o) => {
                let pos = if append { o.borrow().size } else { f.pos };
                let n = o.borrow_mut().write(pos, data);
                f.pos = pos + n as u64;
                Ok(n)
            }
            FileKind::PipeW(p) => {
                let mut p = p.borrow_mut();
                if p.readers == 0 {
                    Err(EPIPE)
                } else {
                    p.buf.extend(data);
                    Ok(data.len())
                }
            }
            FileKind::EventFd(e) => {
                if data.len() < 8 {
                    return Err(EINVAL);
                }
                let v = read_u64_le(data);
                let mut e = e.borrow_mut();
                if v == u64::MAX || e.count.checked_add(v).is_none_or(|n| n == u64::MAX) {
                    return Err(EAGAIN);
                }
                e.count += v;
                Ok(8)
            }
            FileKind::Socket(s) => {
                let s = s.clone();
                drop(f);
                let pid = self.proc(c).pid;
                let r = s.borrow_mut().send(data, Vec::new(), pid);
                r
            }
            FileKind::Dir(_) => Err(EISDIR),
            _ => Err(EBADF),
        };
        if r == Err(EPIPE) {
            let pid = self.proc(c).pid;
            self.sig_send_thread(c, super::signal::SigInfo::user(SIGPIPE, pid));
        }
        r
    }

    pub(crate) fn sys_read(&mut self, c: usize, fd: u64, buf: u64, len: u64) -> Out {
        let file = match self.proc(c).file(fd) {
            Ok(f) => f,
            Err(e) => return Out::Ret(-e),
        };
        let nonblock = file.borrow().flags & O_NONBLOCK != 0;
        match self.file_read(c, &file, (len as usize).min(1 << 20)) {
            Ok(Some(data)) => match self.uwrite(c, buf, &data) {
                Ok(()) => Out::Ret(data.len() as i64),
                Err(e) => Out::Ret(-e),
            },
            Ok(None) if nonblock => Out::Ret(-EAGAIN),
            Ok(None) => Out::Block,
            Err(e) => Out::Ret(-e),
        }
    }

    pub(crate) fn sys_readv(&mut self, c: usize, fd: u64, iov: u64, cnt: u64) -> Out {
        for i in 0..cnt.min(1024) {
            let (base, len) = match (self.uread_u64(c, iov + i * 16), self.uread_u64(c, iov + i * 16 + 8)) {
                (Ok(b), Ok(l)) => (b, l),
                _ => return Out::Ret(-EFAULT),
            };
            if len > 0 {
                return self.sys_read(c, fd, base, len);
            }
        }
        Out::Ret(0)
    }

    pub(crate) fn sys_write(&mut self, c: usize, fd: u64, buf: u64, len: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let data = self.uread(c, buf, (len as usize).min(1 << 20))?;
        self.file_write(c, &file, &data).map(|n| n as i64)
    }

    pub(crate) fn sys_writev(&mut self, c: usize, fd: u64, iov: u64, cnt: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let mut data = Vec::new();
        for i in 0..cnt.min(1024) {
            let base = self.uread_u64(c, iov + i * 16)?;
            let len = self.uread_u64(c, iov + i * 16 + 8)?;
            data.extend(self.uread(c, base, (len as usize).min(1 << 20))?);
        }
        self.file_write(c, &file, &data).map(|n| n as i64)
    }

    fn iovec(&mut self, c: usize, iov: u64, cnt: u64) -> Result<Vec<(u64, u64)>, i64> {
        let mut v = Vec::new();
        for i in 0..cnt.min(1024) {
            v.push((self.uread_u64(c, iov + i * 16)?, self.uread_u64(c, iov + i * 16 + 8)?));
        }
        Ok(v)
    }

    pub(crate) fn sys_preadv(&mut self, c: usize, fd: u64, iov: u64, cnt: u64, off: u64) -> Result<i64, i64> {
        if off as i64 == -1 {
            return match self.sys_readv(c, fd, iov, cnt) {
                super::Out::Ret(v) if v < 0 => Err(-v),
                super::Out::Ret(v) => Ok(v),
                _ => Err(EAGAIN),
            };
        }
        let mut total = 0i64;
        for (base, len) in self.iovec(c, iov, cnt)? {
            if len == 0 {
                continue;
            }
            let n = self.sys_pread(c, fd, base, len, off + total as u64)?;
            total += n;
            if (n as u64) < len {
                break;
            }
        }
        Ok(total)
    }

    pub(crate) fn sys_pwritev(&mut self, c: usize, fd: u64, iov: u64, cnt: u64, off: u64) -> Result<i64, i64> {
        if off as i64 == -1 {
            return self.sys_writev(c, fd, iov, cnt);
        }
        let mut total = 0i64;
        for (base, len) in self.iovec(c, iov, cnt)? {
            if len == 0 {
                continue;
            }
            let n = self.sys_pwrite(c, fd, base, len, off + total as u64)?;
            total += n;
            if (n as u64) < len {
                break;
            }
        }
        Ok(total)
    }

    pub(crate) fn sys_inotify_init1(&mut self, c: usize, flags: u64) -> Result<i64, i64> {
        let e = Rc::new(RefCell::new(EventFd { count: 0, semaphore: false }));
        let f = open_file(FileKind::EventFd(e), flags as u32 & O_NONBLOCK, "anon_inode:inotify");
        self.proc(c).install_fd(f, flags & O_CLOEXEC as u64 != 0, 0).map(|n| n as i64)
    }

    pub(crate) fn sys_pread(&mut self, c: usize, fd: u64, buf: u64, len: u64, off: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let saved = file.borrow().pos;
        file.borrow_mut().pos = off;
        let r = self.file_read(c, &file, len as usize);
        file.borrow_mut().pos = saved;
        let data = r?.ok_or(EAGAIN)?;
        self.uwrite(c, buf, &data).map(|_| data.len() as i64)
    }

    pub(crate) fn sys_pwrite(&mut self, c: usize, fd: u64, buf: u64, len: u64, off: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let data = self.uread(c, buf, len as usize)?;
        let saved = file.borrow().pos;
        file.borrow_mut().pos = off;
        let r = self.file_write(c, &file, &data);
        file.borrow_mut().pos = saved;
        r.map(|n| n as i64)
    }

    pub(crate) fn sys_sendfile(
        &mut self,
        c: usize,
        out_fd: u64,
        in_fd: u64,
        off_ptr: u64,
        count: u64,
    ) -> Result<i64, i64> {
        let (inf, outf) = {
            let p = self.proc(c);
            (p.file(in_fd)?, p.file(out_fd)?)
        };
        let saved = inf.borrow().pos;
        if off_ptr != 0 {
            inf.borrow_mut().pos = self.uread_u64(c, off_ptr)?;
        }
        let data = self.file_read(c, &inf, (count as usize).min(1 << 20))?.ok_or(EAGAIN)?;
        if off_ptr != 0 {
            let newoff = inf.borrow().pos;
            self.uwrite(c, off_ptr, &newoff.to_le_bytes())?;
            inf.borrow_mut().pos = saved;
        }
        self.file_write(c, &outf, &data).map(|n| n as i64)
    }

    pub(crate) fn base_dir(&mut self, c: usize, dirfd: u64) -> Result<String, i64> {
        if dirfd as i32 as i64 == AT_FDCWD {
            return Ok(self.proc(c).cwd.clone());
        }
        let f = self.proc(c).file(dirfd)?;
        let f = f.borrow();
        match f.kind {
            FileKind::Dir(_) => Ok(f.path.clone()),
            _ => Err(ENOTDIR),
        }
    }

    pub(crate) fn sys_openat(&mut self, c: usize, dirfd: u64, path: u64, flags: u32, mode: u32) -> Result<i64, i64> {
        let path = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        let abs = vfs::absolute(&base, &path);
        let procs = self.comps.iter().filter(|x| x.linux.is_some() && !x.free).count();
        let umask = self.proc(c).umask;
        if abs == "/proc/components" {
            let data = self.supervisor("list").into_bytes();
            let f = open_file(FileKind::Mem(data), flags, &abs);
            return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
        }
        // /proc/<pid>: a directory of generated files
        if let Some((pid, tail)) = self.proc_pid_path(c, &abs) {
            // (/proc/self itself stays the static node holding the exe link)
            if tail.is_empty() && !abs.starts_with("/proc/self") {
                let node = super::vfs::Vfs::synthetic_dir(&["stat", "cmdline", "comm", "status"]);
                let f = open_file(FileKind::Dir(node), flags, &abs);
                return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
            }
            if let Some(data) = self.proc_pid_content(pid, tail) {
                let f = open_file(FileKind::Mem(data), flags, &abs);
                return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
            }
        }
        if abs == "/proc/flare-threads" {
            let data = self.proc_threads();
            let f = open_file(FileKind::Mem(data), flags, &abs);
            return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
        }
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
                vfs::DEV_FLARECTL => FileKind::FlareCtl,
                vfs::DEV_FB0 => FileKind::Fb,
                vfs::DEV_FLAREHW => FileKind::FlareHw,
                d if (vfs::DEV_SERIAL0..vfs::DEV_SERIAL0 + 16).contains(&d) => {
                    FileKind::Serial((d - vfs::DEV_SERIAL0) as usize)
                }
                vfs::DEV_DRI_CARD0 => {
                    drop(n);
                    let k = self.drm_open();
                    let f = open_file(k, flags, &r.path);
                    return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
                }
                d if (vfs::DEV_INPUT_EVENT0..vfs::DEV_INPUT_EVENT0 + 32).contains(&d) => {
                    drop(n);
                    let k = self.evdev_open((d - vfs::DEV_INPUT_EVENT0) as usize).ok_or(super::abi::ENODEV)?;
                    let f = open_file(k, flags, &r.path);
                    return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
                }
                vfs::DEV_BINDER | vfs::DEV_HWBINDER | vfs::DEV_VNDBINDER => {
                    let dev = match n.rdev {
                        vfs::DEV_HWBINDER => "hwbinder",
                        vfs::DEV_VNDBINDER => "vndbinder",
                        _ => "binder",
                    };
                    drop(n);
                    let b = self.binder_open(c, dev);
                    let f = open_file(FileKind::Binder(b), flags, &r.path);
                    return self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64);
                }
                vfs::DEV_NULL => FileKind::Null,
                vfs::DEV_ZERO => FileKind::Zero,
                vfs::DEV_URANDOM => FileKind::Random,
                vfs::DEV_KMSG => FileKind::Kmsg,
                _ => FileKind::Tty,
            }
        } else if n.is_reg() {
            FileKind::Reg(node.clone())
        } else if n.mode & S_IFMT == S_IFSOCK {
            return Err(super::abi::ENXIO);
        } else {
            return Err(ELOOP);
        };
        drop(n);
        if flags & O_TRUNC != 0 && flags & O_ACCMODE != 0 {
            if let FileKind::Reg(n) = &kind {
                n.borrow_mut().data.truncate(0);
            }
        }
        let f = open_file(kind, flags, &r.path);
        self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64)
    }

    pub(crate) fn sys_lseek(&mut self, c: usize, fd: u64, off: i64, whence: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let mut f = file.borrow_mut();
        let size = match &f.kind {
            FileKind::Reg(n) => n.borrow().data.len() as i64,
            FileKind::Mem(d) => d.len() as i64,
            FileKind::MemFd(o) => o.borrow().size as i64,
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

    pub(crate) fn sys_getdents(&mut self, c: usize, fd: u64, buf: u64, count: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let out = {
            let mut f = file.borrow_mut();
            let FileKind::Dir(node) = &f.kind else { return Err(ENOTDIR) };
            let dir = node.borrow();
            let mut entries: Vec<(String, u64, u8)> = vec![(".".into(), dir.ino, 4), ("..".into(), 1, 4)];
            if f.path == "/proc" {
                for (pid, _) in self.linux_pids() {
                    entries.push((pid.to_string(), 0x1_0000_0000 + pid as u64, 4));
                }
            }
            for (name, child) in dir.children.iter() {
                let ch = child.borrow();
                let t = match ch.mode & S_IFMT {
                    S_IFDIR => 4,
                    S_IFLNK => 10,
                    S_IFCHR => 2,
                    S_IFIFO => 1,
                    S_IFSOCK => 12,
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
            out
        };
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
            FileKind::Kmsg => stat_bytes(9, S_IFCHR | 0o644, 0, vfs::DEV_KMSG, 0),
            FileKind::PipeR(_) | FileKind::PipeW(_) => stat_bytes(7, S_IFIFO | 0o600, 0, 0, 0),
            FileKind::Mem(d) => stat_bytes(8, S_IFREG | 0o444, d.len() as u64, 0, 0),
            FileKind::MemFd(o) => stat_bytes(10, S_IFREG | 0o600, o.borrow().size, 0, 0),
            FileKind::Socket(_) => stat_bytes(11, S_IFSOCK | 0o777, 0, 0, 0),
            FileKind::Drm(_) => stat_bytes(13, S_IFCHR | 0o666, 0, vfs::DEV_DRI_CARD0, 0),
            FileKind::Serial(d) => stat_bytes(15, S_IFCHR | 0o600, 0, vfs::DEV_SERIAL0 + *d as u64, 0),
            FileKind::Fb => stat_bytes(16, S_IFCHR | 0o660, 0, vfs::DEV_FB0, 0),
            FileKind::Evdev(e) => {
                stat_bytes(14, S_IFCHR | 0o660, 0, vfs::DEV_INPUT_EVENT0 + e.borrow().device as u64, 0)
            }
            _ => stat_bytes(12, 0o600, 0, 0, 0),
        })
    }

    pub(crate) fn sys_fstat(&mut self, c: usize, fd: u64, buf: u64) -> Result<i64, i64> {
        let st = self.fd_stat(c, fd)?;
        self.uwrite(c, buf, &st).map(|_| 0)
    }

    pub(crate) fn sys_fstatat(&mut self, c: usize, dirfd: u64, path: u64, buf: u64, flags: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        if p.is_empty() && flags & AT_EMPTY_PATH != 0 {
            return self.sys_fstat(c, dirfd, buf);
        }
        let base = self.base_dir(c, dirfd)?;
        let abs = vfs::absolute(&base, &p);
        if let Some((pid, tail)) = self.proc_pid_path(c, &abs) {
            if !abs.starts_with("/proc/self") || !tail.is_empty() {
                let st = if tail.is_empty() {
                    stat_bytes(0x1_0000_0000 + pid as u64, S_IFDIR | 0o555, 0, 0, 0)
                } else if tail == "exe" {
                    stat_bytes(0x2_0000_0000 + pid as u64, S_IFLNK | 0o777, 0, 0, 0)
                } else {
                    let n = self.proc_pid_content(pid, tail).ok_or(ENOENT)?.len();
                    stat_bytes(0x3_0000_0000 + pid as u64, S_IFREG | 0o444, n as u64, 0, 0)
                };
                return self.uwrite(c, buf, &st).map(|_| 0);
            }
        }
        let r = self.lx().vfs.lookup(&base, &p, flags & AT_SYMLINK_NOFOLLOW == 0)?;
        let st = node_stat(&r.node);
        self.uwrite(c, buf, &st).map(|_| 0)
    }

    pub(crate) fn sys_fchmod(&mut self, c: usize, fd: u64, mode: u32) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let f = file.borrow();
        match &f.kind {
            FileKind::Reg(n) | FileKind::Dir(n) => {
                let mut n = n.borrow_mut();
                n.mode = (n.mode & S_IFMT) | (mode & 0o7777);
                Ok(0)
            }
            _ => Ok(0),
        }
    }

    pub(crate) fn sys_fchmodat(&mut self, c: usize, dirfd: u64, path: u64, mode: u32) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        let r = self.lx().vfs.lookup(&base, &p, true)?;
        let mut n = r.node.borrow_mut();
        if n.mode & S_IFMT != S_IFCHR {
            n.mode = (n.mode & S_IFMT) | (mode & 0o7777);
        }
        Ok(0)
    }

    pub(crate) fn sys_faccessat(&mut self, c: usize, dirfd: u64, path: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        if vfs::absolute(&base, &p).starts_with("/proc/self/fd/") {
            return Err(ENOENT);
        }
        self.lx().vfs.lookup(&base, &p, true).map(|_| 0)
    }

    pub(crate) fn sys_readlinkat(&mut self, c: usize, dirfd: u64, path: u64, buf: u64, size: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let base = self.base_dir(c, dirfd)?;
        let abs = vfs::absolute(&base, &p);
        let target = if abs == "/proc/self/exe" {
            self.proc(c).exe.clone().into_bytes()
        } else if let Some((pid, "exe")) = self.proc_pid_path(c, &abs) {
            let (_, pc) = *self.linux_pids().iter().find(|(p, _)| *p == pid).ok_or(ENOENT)?;
            self.comps[pc].linux.as_ref().unwrap().p.borrow().exe.clone().into_bytes()
        } else {
            let r = self.lx().vfs.lookup(&base, &p, false)?;
            let n = r.node.borrow();
            if !n.is_symlink() {
                return Err(EINVAL);
            }
            n.data.to_vec()
        };
        let n = target.len().min(size as usize);
        self.uwrite(c, buf, &target[..n]).map(|_| n as i64)
    }

    pub(crate) fn sys_chdir(&mut self, c: usize, path: u64) -> Result<i64, i64> {
        let p = self.ustr(c, path)?;
        let cwd = self.proc(c).cwd.clone();
        let r = self.lx().vfs.lookup(&cwd, &p, true)?;
        if !r.node.borrow().is_dir() {
            return Err(ENOTDIR);
        }
        self.proc(c).cwd = r.path;
        Ok(0)
    }

    pub(crate) fn sys_path2(
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

    pub(crate) fn sys_rename(&mut self, c: usize, ofd: u64, op: u64, nfd: u64, np: u64) -> Result<i64, i64> {
        let (o, n) = (self.ustr(c, op)?, self.ustr(c, np)?);
        let (ob, nb) = (self.base_dir(c, ofd)?, self.base_dir(c, nfd)?);
        let to = vfs::absolute(&nb, &n);
        self.lx().vfs.rename(&ob, &o, &to).map(|_| 0)
    }

    pub(crate) fn sys_link(&mut self, c: usize, ofd: u64, op: u64, nfd: u64, np: u64, flags: u64) -> Result<i64, i64> {
        let (o, n) = (self.ustr(c, op)?, self.ustr(c, np)?);
        let (ob, nb) = (self.base_dir(c, ofd)?, self.base_dir(c, nfd)?);
        let to = vfs::absolute(&nb, &n);
        let follow = flags & 0x400 != 0; // AT_SYMLINK_FOLLOW
        self.lx().vfs.link(&ob, &o, &to, follow).map(|_| 0)
    }

    pub(crate) fn sys_ftruncate(&mut self, c: usize, fd: u64, len: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        let f = file.borrow();
        match &f.kind {
            FileKind::Reg(n) => {
                n.borrow_mut().data.truncate(len as usize);
                Ok(0)
            }
            FileKind::MemFd(o) => {
                o.borrow_mut().truncate(len);
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn sys_dup3(&mut self, c: usize, old: u64, new: u64, flags: u32) -> Result<i64, i64> {
        let mut p = self.proc(c);
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

    pub(crate) fn sys_fcntl(&mut self, c: usize, fd: u64, cmd: u64, arg: u64) -> Result<i64, i64> {
        let mut p = self.proc(c);
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

    pub(crate) fn sys_ioctl(&mut self, c: usize, fd: u64, req: u64, arg: u64) -> Result<i64, i64> {
        let file = self.proc(c).file(fd)?;
        if req == FIONBIO {
            let on = self.uread(c, arg, 4).map(|b| b != [0, 0, 0, 0]).unwrap_or(false);
            let mut f = file.borrow_mut();
            f.flags = if on { f.flags | O_NONBLOCK } else { f.flags & !O_NONBLOCK };
            return Ok(0);
        }
        if req == FIONREAD {
            let n = match &file.borrow().kind {
                FileKind::Tty => self.linux.as_ref().unwrap().tty.pending() as u32,
                FileKind::PipeR(p) => p.borrow().buf.len() as u32,
                FileKind::Socket(s) => s.borrow().pending() as u32,
                _ => 0,
            };
            return self.uwrite(c, arg, &n.to_le_bytes()).map(|_| 0);
        }
        if matches!(file.borrow().kind, FileKind::FlareHw) {
            return self.flarehw_ioctl(c, req, arg);
        }
        if matches!(file.borrow().kind, FileKind::Fb) {
            return self.fb_ioctl(c, req, arg);
        }
        let dev = match &file.borrow().kind {
            FileKind::Drm(d) => Some(Ok(d.clone())),
            FileKind::Evdev(e) => Some(Err(e.clone())),
            _ => None,
        };
        match dev {
            Some(Ok(d)) => return self.drm_ioctl(c, &d, req, arg),
            Some(Err(e)) => return self.evdev_ioctl(c, &e, req, arg),
            None => {}
        }
        if !matches!(file.borrow().kind, FileKind::Tty) {
            return Err(ENOTTY);
        }
        match req {
            TCGETS => {
                let t = self.lx().tty.termios.to_bytes();
                self.uwrite(c, arg, &t).map(|_| 0)
            }
            TCSETS | TCSETSW | TCSETSF => {
                let b = self.uread(c, arg, super::tty::TERMIOS_SIZE)?;
                self.lx().tty.termios = super::tty::Termios::from_bytes(&b);
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
            TIOCSCTTY | TIOCNOTTY => Ok(0),
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn sys_pipe2(&mut self, c: usize, fds: u64, flags: u32) -> Result<i64, i64> {
        let pipe = Rc::new(RefCell::new(Pipe { buf: VecDeque::new(), readers: 1, writers: 1 }));
        let r = open_file(FileKind::PipeR(pipe.clone()), flags & O_NONBLOCK, "pipe:");
        let w = open_file(FileKind::PipeW(pipe), O_WRONLY | (flags & O_NONBLOCK), "pipe:");
        let pair = self.install_pair(c, r, w, flags & O_CLOEXEC != 0)?;
        self.uwrite(c, fds, &pair).map(|_| 0)
    }

    pub(crate) fn install_pair(&mut self, c: usize, a: FileRef, b: FileRef, cloexec: bool) -> Result<[u8; 8], i64> {
        let mut p = self.proc(c);
        let fa = p.install_fd(a, cloexec, 0)?;
        let fb = match p.install_fd(b, cloexec, 0) {
            Ok(n) => n,
            Err(e) => {
                p.fds[fa] = None;
                return Err(e);
            }
        };
        let mut out = [0u8; 8];
        out[..4].copy_from_slice(&(fa as u32).to_le_bytes());
        out[4..].copy_from_slice(&(fb as u32).to_le_bytes());
        Ok(out)
    }

    fn install(&mut self, c: usize, kind: FileKind, flags: u32, name: &str) -> Result<i64, i64> {
        let f = open_file(kind, flags & O_NONBLOCK | O_RDWR, name);
        self.proc(c).install_fd(f, flags & O_CLOEXEC != 0, 0).map(|n| n as i64)
    }

    pub(crate) fn sys_eventfd2(&mut self, c: usize, init: u64, flags: u64) -> Result<i64, i64> {
        let e = EventFd { count: init & 0xffff_ffff, semaphore: flags & 1 != 0 };
        self.install(c, FileKind::EventFd(Rc::new(RefCell::new(e))), flags as u32, "anon_inode:[eventfd]")
    }

    pub(crate) fn sys_memfd_create(&mut self, c: usize, name: u64, flags: u64) -> Result<i64, i64> {
        let n = self.ustr(c, name)?;
        let obj = Rc::new(RefCell::new(MemObj::new(0)));
        let f = open_file(FileKind::MemFd(obj), O_RDWR, &alloc::format!("/memfd:{}", n));
        self.proc(c).install_fd(f, flags & 1 != 0, 0).map(|n| n as i64)
    }

    pub(crate) fn sys_timerfd_create(&mut self, c: usize, clock: u64, flags: u64) -> Result<i64, i64> {
        let t = TimerFd { realtime: clock == 0, deadline: None, interval: 0, ticks: 0 };
        self.install(c, FileKind::TimerFd(Rc::new(RefCell::new(t))), flags as u32, "anon_inode:[timerfd]")
    }

    fn timerfd(&mut self, c: usize, fd: u64) -> Result<Rc<RefCell<TimerFd>>, i64> {
        let f = self.proc(c).file(fd)?;
        let f = f.borrow();
        match &f.kind {
            FileKind::TimerFd(t) => Ok(t.clone()),
            _ => Err(EINVAL),
        }
    }

    fn itimerspec(t: &TimerFd, now: u64) -> [u8; 32] {
        let rem = t.deadline.map(|d| d.saturating_sub(now)).unwrap_or(0);
        let mut b = [0u8; 32];
        b[0..8].copy_from_slice(&(t.interval / 1_000_000_000).to_le_bytes());
        b[8..16].copy_from_slice(&(t.interval % 1_000_000_000).to_le_bytes());
        b[16..24].copy_from_slice(&(rem / 1_000_000_000).to_le_bytes());
        b[24..32].copy_from_slice(&(rem % 1_000_000_000).to_le_bytes());
        b
    }

    pub(crate) fn sys_timerfd_settime(
        &mut self,
        c: usize,
        fd: u64,
        flags: u64,
        new: u64,
        old: u64,
    ) -> Result<i64, i64> {
        let t = self.timerfd(c, fd)?;
        let now = timer::now_ns();
        if old != 0 {
            let b = Self::itimerspec(&t.borrow(), now);
            self.uwrite(c, old, &b)?;
        }
        let b = self.uread(c, new, 32)?;
        let g = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        let interval = g(0) * 1_000_000_000 + g(1);
        let value = g(2) * 1_000_000_000 + g(3);
        let epoch = self.lx().epoch_ns();
        let mut t = t.borrow_mut();
        t.interval = interval;
        t.ticks = 0;
        t.deadline = if value == 0 {
            None
        } else if flags & 1 != 0 {
            Some(if t.realtime { value.saturating_sub(epoch) } else { value })
        } else {
            Some(now + value)
        };
        Ok(0)
    }

    pub(crate) fn sys_timerfd_gettime(&mut self, c: usize, fd: u64, cur: u64) -> Result<i64, i64> {
        let t = self.timerfd(c, fd)?;
        let b = Self::itimerspec(&t.borrow(), timer::now_ns());
        self.uwrite(c, cur, &b).map(|_| 0)
    }

    pub(crate) fn sys_signalfd4(&mut self, c: usize, fd: u64, mask: u64, flags: u64) -> Result<i64, i64> {
        let m = self.uread_u64(c, mask)?;
        if fd as i32 != -1 {
            let f = self.proc(c).file(fd)?;
            let mut f = f.borrow_mut();
            return match &mut f.kind {
                FileKind::SignalFd(old) => {
                    *old = m;
                    Ok(fd as i64)
                }
                _ => Err(EINVAL),
            };
        }
        self.install(c, FileKind::SignalFd(m), flags as u32, "anon_inode:[signalfd]")
    }

    pub(crate) fn sys_epoll_create1(&mut self, c: usize, flags: u64) -> Result<i64, i64> {
        let e = Epoll { entries: Vec::new() };
        self.install(c, FileKind::Epoll(Rc::new(RefCell::new(e))), flags as u32, "anon_inode:[eventpoll]")
    }

    fn epoll(&mut self, c: usize, fd: u64) -> Result<Rc<RefCell<Epoll>>, i64> {
        let f = self.proc(c).file(fd)?;
        let f = f.borrow();
        match &f.kind {
            FileKind::Epoll(e) => Ok(e.clone()),
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn sys_epoll_ctl(&mut self, c: usize, epfd: u64, op: u64, fd: u64, ev: u64) -> Result<i64, i64> {
        let ep = self.epoll(c, epfd)?;
        let file = self.proc(c).file(fd)?;
        let (events, data) = if ev != 0 && op != 2 {
            let b = self.uread(c, ev, 16)?;
            (u32::from_le_bytes(b[0..4].try_into().unwrap()), u64::from_le_bytes(b[8..16].try_into().unwrap()))
        } else {
            (0, 0)
        };
        let mut ep = ep.borrow_mut();
        ep.entries.retain(|e| e.file.strong_count() > 0);
        let pos = ep
            .entries
            .iter()
            .position(|e| e.fd == fd as i32 && e.file.upgrade().is_some_and(|f| Rc::ptr_eq(&f, &file)));
        match (op, pos) {
            (1, Some(_)) => Err(EEXIST),
            (1, None) => {
                ep.entries.push(EpollEntry {
                    fd: fd as i32,
                    file: Rc::downgrade(&file),
                    events,
                    data,
                    disabled: false,
                });
                Ok(0)
            }
            (2, Some(i)) => {
                ep.entries.remove(i);
                Ok(0)
            }
            (3, Some(i)) => {
                let e = &mut ep.entries[i];
                e.events = events;
                e.data = data;
                e.disabled = false;
                Ok(0)
            }
            (2 | 3, None) => Err(ENOENT),
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn sys_epoll_pwait(&mut self, c: usize, epfd: u64, out: u64, max: u64, timeout: i64) -> Out {
        let ep = match self.epoll(c, epfd) {
            Ok(e) => e,
            Err(e) => return Out::Ret(-e),
        };
        if max == 0 || max > 1024 {
            return Out::Ret(-EINVAL);
        }
        let candidates: Vec<(usize, FileRef, u32, u64)> = {
            let mut e = ep.borrow_mut();
            e.entries.retain(|x| x.file.strong_count() > 0);
            e.entries
                .iter()
                .enumerate()
                .filter(|(_, x)| !x.disabled)
                .filter_map(|(i, x)| x.file.upgrade().map(|f| (i, f, x.events, x.data)))
                .collect()
        };
        let mut ready = Vec::new();
        for (i, f, events, data) in candidates {
            let rev = self.file_poll(c, &f) & (events | EPOLLERR | EPOLLHUP);
            if rev != 0 && ready.len() < max as usize {
                ready.push((i, rev, data, events));
            }
        }
        if ready.is_empty() {
            let now = timer::now_ns();
            let expired = match timeout {
                0 => true,
                t if t < 0 => false,
                t => {
                    let d = *self.task(c).wait.deadline.get_or_insert(now + t as u64 * 1_000_000);
                    now >= d
                }
            };
            if !expired {
                return Out::Block;
            }
        }
        self.task(c).wait = Default::default();
        let mut buf = Vec::new();
        {
            let mut e = ep.borrow_mut();
            for &(i, rev, data, events) in &ready {
                buf.extend_from_slice(&rev.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(&data.to_le_bytes());
                if events & EPOLLONESHOT != 0 {
                    if let Some(x) = e.entries.get_mut(i) {
                        x.disabled = true;
                    }
                }
            }
        }
        match self.uwrite(c, out, &buf) {
            Ok(()) => Out::Ret(ready.len() as i64),
            Err(e) => Out::Ret(-e),
        }
    }

    pub(crate) fn sys_ppoll(&mut self, c: usize, fds: u64, nfds: u64, tmo: u64) -> Out {
        let raw = match self.uread(c, fds, (nfds.min(1024) * 8) as usize) {
            Ok(r) => r,
            Err(e) => return Out::Ret(-e),
        };
        let mut out = raw.clone();
        let mut ready = 0;
        for i in 0..nfds.min(1024) as usize {
            let fd = i32::from_le_bytes(raw[i * 8..i * 8 + 4].try_into().unwrap());
            let events = u16::from_le_bytes([raw[i * 8 + 4], raw[i * 8 + 5]]) as u32;
            let rev = if fd < 0 {
                0
            } else {
                match self.proc(c).file(fd as u64) {
                    Err(_) => POLLNVAL as u32,
                    Ok(f) => self.file_poll(c, &f) & (events | EPOLLHUP | EPOLLERR),
                }
            };
            out[i * 8 + 6..i * 8 + 8].copy_from_slice(&(rev as u16).to_le_bytes());
            if rev != 0 {
                ready += 1;
            }
        }
        let timeout_ns = if tmo == 0 {
            None
        } else {
            match (self.uread_u64(c, tmo), self.uread_u64(c, tmo + 8)) {
                (Ok(s), Ok(ns)) => Some(s.saturating_mul(1_000_000_000).saturating_add(ns)),
                _ => return Out::Ret(-EFAULT),
            }
        };
        let expired = match timeout_ns {
            Some(0) => true,
            Some(t) => {
                let now = timer::now_ns();
                let d = *self.task(c).wait.deadline.get_or_insert(now + t);
                now >= d
            }
            None => false,
        };
        if ready > 0 || expired {
            self.task(c).wait = Default::default();
            if let Err(e) = self.uwrite(c, fds, &out) {
                return Out::Ret(-e);
            }
            return Out::Ret(ready);
        }
        Out::Block
    }
}
