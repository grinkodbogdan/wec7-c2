//! Linux's drivers for every program: files and sockets of the driver
//! host's Linux.
//!
//! The driver host (`linuxdrv`, a Linux kernel as a library) drives the
//! phone's hardware. Its devices reach Flare's programs as ordinary files
//! and sockets, served by that Linux:
//!
//! * `/linux/PATH` is its `PATH` (`/linux/sys/...`, `/linux/dev/...`);
//!   `/sys/class/{power_supply,backlight,leds,net,rfkill}`, `/proc/net` and
//!   `/dev/snd` are its own under their usual names, so battery, brightness,
//!   network and sound programs work unchanged;
//! * sockets of the Internet (IPv4, IPv6), netlink and packet families are
//!   its sockets: its network stack, its Wi-Fi.
//!
//! Each system call on such a file becomes a request the driver host reads
//! from `/dev/flarelk`; it makes the same call on its Linux and writes the
//! reply back. The caller waits meanwhile (the call is re-run until its
//! reply is there). Pointers are never passed: the kernel copies what a
//! call reads and writes (ioctls by the size in their number, interface
//! ioctls as `struct ifreq`, ALSA's frame transfers by the stream's frame
//! size). The host also reports each file's readiness (poll events), so
//! poll, epoll and select work on them.

use super::abi::*;
use super::fs::{open_file, FileKind, FileRef};
use super::Kernel;
use super::Out;
use crate::global::Global;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

/// Operations (the request's `op`).
pub const OP_OPEN: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_READ: u32 = 3;
pub const OP_WRITE: u32 = 4;
pub const OP_IOCTL: u32 = 5;
pub const OP_FSTAT: u32 = 6;
pub const OP_LSEEK: u32 = 7;
pub const OP_GETDENTS: u32 = 8;
pub const OP_SOCKET: u32 = 9;
pub const OP_CONNECT: u32 = 10;
pub const OP_BIND: u32 = 11;
pub const OP_LISTEN: u32 = 12;
pub const OP_ACCEPT: u32 = 13;
pub const OP_SENDTO: u32 = 14;
pub const OP_RECVFROM: u32 = 15;
pub const OP_GETSOCKOPT: u32 = 16;
pub const OP_SETSOCKOPT: u32 = 17;
pub const OP_SOCKNAME: u32 = 18;
pub const OP_SHUTDOWN: u32 = 19;
pub const OP_SETFL: u32 = 20;
pub const OP_STAT: u32 = 21;
pub const OP_ACCESS: u32 = 22;
pub const OP_READLINK: u32 = 23;

/// What a call may move in one request.
const MAX_IO: usize = 1 << 20;

const AF_INET: u64 = 2;
const AF_INET6: u64 = 10;
const AF_NETLINK: u64 = 16;
const AF_PACKET: u64 = 17;

/// ALSA's interleaved frame transfers, and the hardware parameters that
/// give a stream's frame size.
const SNDRV_PCM_IOCTL_HW_PARAMS: u64 = 0xc260_4111;
const SNDRV_PCM_IOCTL_WRITEI_FRAMES: u64 = 0x4018_4150;
const SNDRV_PCM_IOCTL_READI_FRAMES: u64 = 0x8018_4151;
const SO_ATTACH_FILTER: u64 = 26;
const FIONBIO: u64 = 0x5421;

/// A file or socket of the driver host's Linux, by its descriptor there.
pub struct RemoteFile {
    pub handle: i32,
    /// Poll events, as the host last reported them.
    pub events: u32,
    /// ALSA: bytes per frame (from HW_PARAMS).
    pub frame_bytes: u32,
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        // closed on the host by the next request the host reads
        // SAFETY: under the kernel lock.
        unsafe { CLOSED.get().push(self.handle) };
    }
}

pub type RemoteRef = Rc<RefCell<RemoteFile>>;

/// Descriptors to close on the host.
static CLOSED: Global<Vec<i32>> = Global::new(Vec::new());

/// The personality's side of the channel.
#[derive(Default)]
pub struct Remote {
    /// The driver host reads `/dev/flarelk`.
    pub serving: bool,
    queue: VecDeque<Vec<u8>>,
    next_id: u64,
    /// Requests whose caller still waits.
    waiting: BTreeSet<u64>,
    done: BTreeMap<u64, (i64, Vec<u8>)>,
    files: BTreeMap<i32, Weak<RefCell<RemoteFile>>>,
}

/// Whether `path` (absolute) is the driver host's, and its path there.
pub fn remote_path(path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix("/linux") {
        if rest.is_empty() {
            return Some("/".into());
        }
        if rest.starts_with('/') {
            return Some(rest.into());
        }
        return None;
    }
    const OWN: [&str; 7] = [
        "/sys/class/power_supply",
        "/sys/class/backlight",
        "/sys/class/leds",
        "/sys/class/net",
        "/sys/class/rfkill",
        "/proc/net",
        "/dev/snd",
    ];
    OWN.iter()
        .any(|p| path == *p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/')))
        .then(|| path.into())
}

fn header(id: u64, op: u32, handle: i32, a: [u64; 3], len: usize) -> Vec<u8> {
    let mut b = Vec::with_capacity(48 + len);
    b.extend_from_slice(&id.to_le_bytes());
    b.extend_from_slice(&op.to_le_bytes());
    b.extend_from_slice(&handle.to_le_bytes());
    for x in a {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.extend_from_slice(&(len as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

impl Kernel {
    fn rm(&mut self) -> &mut Remote {
        &mut self.lx().remote
    }

    /// The call `sig` of task `c`: its reply once there (`Some`), else the
    /// request is queued (once) and the caller waits (`None`).
    fn remote_call(&mut self, c: usize, sig: u64, op: u32, handle: i32, a: [u64; 3], data: &[u8]) -> Option<(i64, Vec<u8>)> {
        if !self.lx().remote.serving {
            return Some((-ENODEV, Vec::new()));
        }
        let pending = self.task(c).remote;
        if let Some((id, s)) = pending {
            if s == sig {
                let r = self.rm().done.remove(&id);
                if r.is_some() {
                    self.task(c).remote = None;
                }
                return r;
            }
            // a call given up (a signal): its reply is dropped
            self.rm().waiting.remove(&id);
            self.rm().done.remove(&id);
        }
        let rm = self.rm();
        rm.next_id += 1;
        let id = rm.next_id;
        let mut req = header(id, op, handle, a, data.len());
        req.extend_from_slice(data);
        rm.queue.push_back(req);
        rm.waiting.insert(id);
        self.task(c).remote = Some((id, sig));
        None
    }

    fn new_remote(&mut self, handle: i32, flags: u32, path: &str) -> FileRef {
        let r = Rc::new(RefCell::new(RemoteFile { handle, events: 0, frame_bytes: 0 }));
        self.rm().files.insert(handle, Rc::downgrade(&r));
        open_file(FileKind::Remote(r), flags, path)
    }

    /// `/dev/flarelk`, read by the host: the next request (closes first).
    pub(crate) fn flarelk_read(&mut self, max: usize) -> Option<Vec<u8>> {
        // SAFETY: under the kernel lock.
        let closed = unsafe { CLOSED.get() };
        let rm = &mut self.lx().remote;
        rm.serving = true;
        if let Some(h) = closed.pop() {
            rm.files.remove(&h);
            return Some(header(0, OP_CLOSE, h, [0; 3], 0));
        }
        match rm.queue.front() {
            Some(r) if r.len() <= max => rm.queue.pop_front(),
            Some(_) => {
                // (the host reads with a buffer that takes any request)
                let r = rm.queue.pop_front().unwrap();
                Some(r[..max].to_vec())
            }
            None => None,
        }
    }

    /// `/dev/flarelk`, written by the host: replies and readiness.
    /// Header: u64 id (or handle), i64 result (or events), u32 length, u32
    /// kind (0: a reply, 1: readiness); then the reply's data.
    pub(crate) fn flarelk_write(&mut self, mut b: &[u8]) -> usize {
        let total = b.len();
        while b.len() >= 24 {
            let id = u64::from_le_bytes(b[0..8].try_into().unwrap());
            let res = i64::from_le_bytes(b[8..16].try_into().unwrap());
            let len = u32::from_le_bytes(b[16..20].try_into().unwrap()) as usize;
            let kind = u32::from_le_bytes(b[20..24].try_into().unwrap());
            if b.len() < 24 + len {
                break;
            }
            let data = &b[24..24 + len];
            let rm = &mut self.lx().remote;
            match kind {
                0 if rm.waiting.remove(&id) => {
                    rm.done.insert(id, (res, data.to_vec()));
                }
                1 => {
                    if let Some(f) = rm.files.get(&(id as i32)).and_then(|w| w.upgrade()) {
                        f.borrow_mut().events = res as u32;
                    }
                }
                _ => {}
            }
            b = &b[24 + len..];
        }
        total
    }

    pub(crate) fn flarelk_readable(&self) -> bool {
        // SAFETY: under the kernel lock.
        let closed = unsafe { CLOSED.get() };
        self.linux.as_ref().is_some_and(|l| !l.remote.queue.is_empty()) || !closed.is_empty()
    }

    /// System calls on the driver host's files and sockets; `None` for any
    /// other call.
    pub(crate) fn remote_dispatch(&mut self, c: usize, nr_: u64, a: &[u64; 6]) -> Option<Out> {
        let sig = {
            let mut h = nr_.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            for x in a {
                h = (h ^ x).wrapping_mul(0x0100_0000_01b3);
            }
            h
        };
        let ret = |v: Result<i64, i64>| Some(Out::Ret(v.unwrap_or_else(|e| -e)));
        // calls naming a path
        let path_arg = match nr_ {
            nr::OPENAT | nr::NEWFSTATAT | nr::FACCESSAT | nr::FACCESSAT2 | nr::READLINKAT | nr::STATX => Some(a[1]),
            _ => None,
        };
        if let Some(p) = path_arg {
            if !self.lx().remote.serving {
                return None;
            }
            let path = self.ustr(c, p).ok()?;
            let base = self.base_dir(c, a[0]).ok()?;
            let abs = super::vfs::absolute(&base, &path);
            let rpath = remote_path(&abs)?;
            let mut d = rpath.clone().into_bytes();
            d.push(0);
            return Some(match nr_ {
                nr::OPENAT => match self.remote_call(c, sig, OP_OPEN, -1, [a[2] & !0o2000000, a[3], 0], &d) {
                    None => Out::Block,
                    Some((h, _)) if h < 0 => Out::Ret(h),
                    Some((h, _)) => {
                        let f = self.new_remote(h as i32, a[2] as u32, &abs);
                        let r = self.proc(c).install_fd(f, a[2] as u32 & O_CLOEXEC != 0, 0).map(|n| n as i64);
                        Out::Ret(r.unwrap_or_else(|e| -e))
                    }
                },
                nr::NEWFSTATAT => match self.remote_call(c, sig, OP_STAT, -1, [a[3], 0, 0], &d) {
                    None => Out::Block,
                    Some((r, st)) if r == 0 => Out::Ret(self.uwrite(c, a[2], &st).map(|_| 0).unwrap_or_else(|e| -e)),
                    Some((r, _)) => Out::Ret(r),
                },
                nr::STATX => Out::Ret(-ENOSYS), // musl falls back to fstatat
                nr::READLINKAT => match self.remote_call(c, sig, OP_READLINK, -1, [a[3].min(4096), 0, 0], &d) {
                    None => Out::Block,
                    Some((r, s)) if r >= 0 => {
                        Out::Ret(self.uwrite(c, a[2], &s).map(|_| s.len() as i64).unwrap_or_else(|e| -e))
                    }
                    Some((r, _)) => Out::Ret(r),
                },
                _ => match self.remote_call(c, sig, OP_ACCESS, -1, [a[2], 0, 0], &d) {
                    None => Out::Block,
                    Some((r, _)) => Out::Ret(r),
                },
            });
        }
        // socket(): the Internet, netlink and packet families are the host's
        if nr_ == nr::SOCKET {
            if ![AF_INET, AF_INET6, AF_NETLINK, AF_PACKET].contains(&a[0]) || !self.lx().remote.serving {
                return None;
            }
            return Some(match self.remote_call(c, sig, OP_SOCKET, -1, [a[0], a[1], a[2]], &[]) {
                None => Out::Block,
                Some((h, _)) if h < 0 => Out::Ret(h),
                Some((h, _)) => {
                    let flags = O_RDWR | (a[1] as u32 & O_NONBLOCK);
                    let f = self.new_remote(h as i32, flags, "socket:");
                    Out::Ret(self.proc(c).install_fd(f, a[1] as u32 & O_CLOEXEC != 0, 0).map(|n| n as i64).unwrap_or_else(|e| -e))
                }
            });
        }
        // calls on a descriptor
        let on_fd = matches!(
            nr_,
            nr::READ
                | nr::WRITE
                | nr::READV
                | nr::WRITEV
                | nr::PREAD64
                | nr::PWRITE64
                | nr::IOCTL
                | nr::FSTAT
                | nr::LSEEK
                | nr::GETDENTS64
                | nr::CONNECT
                | nr::BIND
                | nr::LISTEN
                | nr::ACCEPT
                | nr::ACCEPT4
                | nr::SENDTO
                | nr::RECVFROM
                | nr::SENDMSG
                | nr::RECVMSG
                | nr::GETSOCKOPT
                | nr::SETSOCKOPT
                | nr::GETSOCKNAME
                | nr::GETPEERNAME
                | nr::SHUTDOWN
                | nr::FCNTL
                | nr::MMAP
        );
        if !on_fd {
            return None;
        }
        let file = self.proc(c).file(a[if nr_ == nr::MMAP { 4 } else { 0 }]).ok()?;
        let r = match &file.borrow().kind {
            FileKind::Remote(r) => r.clone(),
            _ => return None,
        };
        let h = r.borrow().handle;
        let nonblock = file.borrow().flags & O_NONBLOCK != 0;
        // a reply that says "would block" on a blocking file: wait for
        // readiness, then ask again
        let again = |s: &mut Kernel, res: i64| -> Option<Out> {
            if res == -EAGAIN && !nonblock {
                let _ = s;
                return Some(Out::Block);
            }
            None
        };
        macro_rules! call {
            ($op:expr, $args:expr, $data:expr) => {
                match self.remote_call(c, sig, $op, h, $args, $data) {
                    None => return Some(Out::Block),
                    Some(x) => x,
                }
            };
        }
        match nr_ {
            nr::MMAP => ret(Err(ENODEV)),
            nr::READ | nr::PREAD64 | nr::RECVFROM => {
                let (buf, len) = (a[1], (a[2] as usize).min(MAX_IO));
                let off = if nr_ == nr::PREAD64 { a[3] } else { u64::MAX };
                let (op, args) = if nr_ == nr::RECVFROM { (OP_RECVFROM, [len as u64, a[3], 0]) } else { (OP_READ, [len as u64, off, 0]) };
                let (res, data) = call!(op, args, &[]);
                if let Some(o) = again(self, res) {
                    return Some(o);
                }
                if res < 0 {
                    return ret(Ok(res));
                }
                // recvfrom: u32 address length, the address, then the bytes
                let (addr, body) = if nr_ == nr::RECVFROM && data.len() >= 4 {
                    let al = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
                    (Some(data[4..4 + al].to_vec()), data[4 + al..].to_vec())
                } else {
                    (None, data)
                };
                if self.uwrite(c, buf, &body).is_err() {
                    return ret(Err(EFAULT));
                }
                if let (Some(addr), true) = (addr, a[4] != 0 && a[5] != 0) {
                    let cap = self.uread(c, a[5], 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0) as usize;
                    let _ = self.uwrite(c, a[4], &addr[..addr.len().min(cap)]);
                    let _ = self.uwrite(c, a[5], &(addr.len() as u32).to_le_bytes());
                }
                ret(Ok(res))
            }
            nr::WRITE | nr::PWRITE64 | nr::SENDTO => {
                let len = (a[2] as usize).min(MAX_IO);
                let Ok(mut data) = self.uread(c, a[1], len) else { return ret(Err(EFAULT)) };
                let (op, args) = if nr_ == nr::SENDTO {
                    // the address after the bytes
                    let alen = if a[4] != 0 { (a[5] as usize).min(128) } else { 0 };
                    if alen > 0 {
                        let Ok(ad) = self.uread(c, a[4], alen) else { return ret(Err(EFAULT)) };
                        data.extend_from_slice(&ad);
                    }
                    (OP_SENDTO, [len as u64, a[3], alen as u64])
                } else {
                    (OP_WRITE, [len as u64, if nr_ == nr::PWRITE64 { a[3] } else { u64::MAX }, 0])
                };
                let (res, _) = call!(op, args, &data);
                if let Some(o) = again(self, res) {
                    return Some(o);
                }
                ret(Ok(res))
            }
            nr::READV | nr::WRITEV => {
                // the first non-empty buffer (as the local files do)
                for i in 0..a[2].min(1024) {
                    let (Ok(base), Ok(len)) = (self.uread_u64(c, a[1] + i * 16), self.uread_u64(c, a[1] + i * 16 + 8)) else {
                        return ret(Err(EFAULT));
                    };
                    if len > 0 {
                        let n = if nr_ == nr::READV { nr::READ } else { nr::WRITE };
                        return self.remote_dispatch(c, n, &[a[0], base, len, 0, 0, 0]);
                    }
                }
                ret(Ok(0))
            }
            nr::SENDMSG | nr::RECVMSG => self.remote_msg(c, sig, &r, nr_ == nr::SENDMSG, a, nonblock),
            nr::IOCTL => self.remote_ioctl(c, sig, &r, a[1], a[2], &file),
            nr::FSTAT => {
                let (res, st) = call!(OP_FSTAT, [0; 3], &[]);
                if res < 0 {
                    return ret(Ok(res));
                }
                ret(self.uwrite(c, a[1], &st).map(|_| 0))
            }
            nr::LSEEK => ret(Ok(call!(OP_LSEEK, [a[1], a[2], 0], &[]).0)),
            nr::GETDENTS64 => {
                let (res, d) = call!(OP_GETDENTS, [(a[2] as usize).min(MAX_IO) as u64, 0, 0], &[]);
                if res < 0 {
                    return ret(Ok(res));
                }
                ret(self.uwrite(c, a[1], &d).map(|_| d.len() as i64))
            }
            nr::CONNECT | nr::BIND => {
                let Ok(addr) = self.uread(c, a[1], (a[2] as usize).min(128)) else { return ret(Err(EFAULT)) };
                let op = if nr_ == nr::CONNECT { OP_CONNECT } else { OP_BIND };
                let (res, _) = call!(op, [0; 3], &addr);
                if res == -EINPROGRESS || res == -EAGAIN {
                    // a non-blocking connect: under way; a blocking one waits
                    // until the socket is writable
                    if nonblock {
                        return ret(Ok(res));
                    }
                }
                ret(Ok(res))
            }
            nr::LISTEN => ret(Ok(call!(OP_LISTEN, [a[1], 0, 0], &[]).0)),
            nr::SHUTDOWN => ret(Ok(call!(OP_SHUTDOWN, [a[1], 0, 0], &[]).0)),
            nr::ACCEPT | nr::ACCEPT4 => {
                let flags = if nr_ == nr::ACCEPT4 { a[3] } else { 0 };
                let (res, addr) = call!(OP_ACCEPT, [flags, 0, 0], &[]);
                if let Some(o) = again(self, res) {
                    return Some(o);
                }
                if res < 0 {
                    return ret(Ok(res));
                }
                if a[1] != 0 && a[2] != 0 {
                    let cap = self.uread(c, a[2], 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0) as usize;
                    let _ = self.uwrite(c, a[1], &addr[..addr.len().min(cap)]);
                    let _ = self.uwrite(c, a[2], &(addr.len() as u32).to_le_bytes());
                }
                let f = self.new_remote(res as i32, O_RDWR | (flags as u32 & O_NONBLOCK), "socket:");
                ret(self.proc(c).install_fd(f, flags as u32 & O_CLOEXEC != 0, 0).map(|n| n as i64))
            }
            nr::GETSOCKNAME | nr::GETPEERNAME => {
                let (res, addr) = call!(OP_SOCKNAME, [(nr_ == nr::GETPEERNAME) as u64, 0, 0], &[]);
                if res < 0 {
                    return ret(Ok(res));
                }
                let cap = self.uread(c, a[2], 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0) as usize;
                let _ = self.uwrite(c, a[1], &addr[..addr.len().min(cap)]);
                ret(self.uwrite(c, a[2], &(addr.len() as u32).to_le_bytes()).map(|_| 0))
            }
            nr::GETSOCKOPT => {
                let cap = self.uread(c, a[4], 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0).min(256);
                let (res, v) = call!(OP_GETSOCKOPT, [a[1], a[2], cap as u64], &[]);
                if res < 0 {
                    return ret(Ok(res));
                }
                let _ = self.uwrite(c, a[3], &v);
                ret(self.uwrite(c, a[4], &(v.len() as u32).to_le_bytes()).map(|_| 0))
            }
            nr::SETSOCKOPT => {
                let len = (a[4] as usize).min(4096);
                let Ok(mut v) = self.uread(c, a[3], len) else { return ret(Err(EFAULT)) };
                // a socket filter (struct sock_fprog): its instructions
                // instead of the pointer to them
                if a[1] == 1 && a[2] == SO_ATTACH_FILTER && v.len() >= 16 {
                    let n = u16::from_le_bytes([v[0], v[1]]) as usize;
                    let p = u64::from_le_bytes(v[8..16].try_into().unwrap());
                    let Ok(ins) = self.uread(c, p, (n * 8).min(4096)) else { return ret(Err(EFAULT)) };
                    v = (n as u16).to_le_bytes().to_vec();
                    v.extend_from_slice(&ins);
                }
                ret(Ok(call!(OP_SETSOCKOPT, [a[1], a[2], 0], &v).0))
            }
            nr::FCNTL => {
                const F_SETFL: u64 = 4;
                if a[1] != F_SETFL {
                    return None; // the local descriptor's flags serve the rest
                }
                let (res, _) = call!(OP_SETFL, [a[2], 0, 0], &[]);
                if res >= 0 {
                    let mut f = file.borrow_mut();
                    f.flags = (f.flags & !O_NONBLOCK) | (a[2] as u32 & O_NONBLOCK);
                }
                ret(Ok(res.min(0)))
            }
            _ => None,
        }
    }

    fn remote_msg(&mut self, c: usize, sig: u64, r: &RemoteRef, send: bool, a: &[u64; 6], nonblock: bool) -> Option<Out> {
        // struct msghdr: name, namelen, iov, iovlen, control, controllen, flags
        let m = a[1];
        let rd = |s: &mut Kernel, off: u64| s.uread_u64(c, m + off);
        let (Ok(name), Ok(namelen), Ok(iov), Ok(iovlen)) = (rd(self, 0), rd(self, 8), rd(self, 16), rd(self, 24)) else {
            return Some(Out::Ret(-EFAULT));
        };
        let namelen = namelen & 0xffff_ffff;
        let mut bufs = Vec::new();
        for i in 0..iovlen.min(64) {
            let (Ok(b), Ok(l)) = (self.uread_u64(c, iov + i * 16), self.uread_u64(c, iov + i * 16 + 8)) else {
                return Some(Out::Ret(-EFAULT));
            };
            bufs.push((b, l));
        }
        let total: usize = bufs.iter().map(|b| b.1 as usize).sum::<usize>().min(MAX_IO);
        let h = r.borrow().handle;
        if send {
            // as sendto: the bytes (gathered), then the address
            let mut d = Vec::with_capacity(total);
            for (b, l) in &bufs {
                let Ok(x) = self.uread(c, *b, (*l as usize).min(MAX_IO)) else { return Some(Out::Ret(-EFAULT)) };
                d.extend_from_slice(&x);
            }
            d.truncate(total);
            let alen = if name != 0 { (namelen as usize).min(128) } else { 0 };
            if alen > 0 {
                let Ok(ad) = self.uread(c, name, alen) else { return Some(Out::Ret(-EFAULT)) };
                d.extend_from_slice(&ad);
            }
            let (res, _) = match self.remote_call(c, sig, OP_SENDTO, h, [total as u64, a[2], alen as u64], &d) {
                None => return Some(Out::Block),
                Some(x) => x,
            };
            if res == -EAGAIN && !nonblock {
                return Some(Out::Block);
            }
            return Some(Out::Ret(res));
        }
        let (res, data) = match self.remote_call(c, sig, OP_RECVFROM, h, [total as u64, a[2], 1], &[]) {
            None => return Some(Out::Block),
            Some(x) => x,
        };
        if res == -EAGAIN && !nonblock {
            return Some(Out::Block);
        }
        if res < 0 {
            return Some(Out::Ret(res));
        }
        // u32 address length, address, u32 msg flags, the bytes
        let al = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let addr = &data[4..4 + al];
        let flags = u32::from_le_bytes(data[4 + al..8 + al].try_into().unwrap());
        let mut body = &data[8 + al..];
        for (b, l) in &bufs {
            let n = (*l as usize).min(body.len());
            if n == 0 {
                break;
            }
            if self.uwrite(c, *b, &body[..n]).is_err() {
                return Some(Out::Ret(-EFAULT));
            }
            body = &body[n..];
        }
        if name != 0 {
            let n = al.min(namelen as usize);
            let _ = self.uwrite(c, name, &addr[..n]);
            let _ = self.uwrite(c, m + 8, &(al as u32).to_le_bytes());
        }
        let _ = self.uwrite(c, m + 40, &0u64.to_le_bytes()); // no control messages
        let _ = self.uwrite(c, m + 48, &flags.to_le_bytes());
        Some(Out::Ret(res))
    }

    fn remote_ioctl(&mut self, c: usize, sig: u64, r: &RemoteRef, cmd: u64, arg: u64, file: &FileRef) -> Option<Out> {
        let h = r.borrow().handle;
        let cmd = cmd & 0xffff_ffff;
        // what the call reads and writes: by the size in its number, or
        // the classic ones by what they are
        let (dir, size) = ((cmd >> 30) & 3, ((cmd >> 16) & 0x3fff) as usize);
        let (inb, outb) = match cmd {
            FIONBIO => (4, 0),
            0x541b => (0, 4),                                // FIONREAD
            0x8900..=0x89ff if cmd != 0x8912 => (40, 40),    // interface ioctls (struct ifreq)
            SNDRV_PCM_IOCTL_WRITEI_FRAMES | SNDRV_PCM_IOCTL_READI_FRAMES => (0, 0),
            _ if dir != 0 => (if dir & 1 != 0 { size } else { 0 }, if dir & 2 != 0 { size } else { 0 }),
            _ => (0, 0),
        };
        let mut data = Vec::new();
        let mut extra = 0u64;
        if cmd == SNDRV_PCM_IOCTL_WRITEI_FRAMES || cmd == SNDRV_PCM_IOCTL_READI_FRAMES {
            // struct snd_xferi { result; buf; frames }: the frames moved
            let (Ok(buf), Ok(frames)) = (self.uread_u64(c, arg + 8), self.uread_u64(c, arg + 16)) else {
                return Some(Out::Ret(-EFAULT));
            };
            let fb = r.borrow().frame_bytes.max(1) as u64;
            let bytes = (frames * fb).min(MAX_IO as u64);
            extra = bytes / fb;
            if cmd == SNDRV_PCM_IOCTL_WRITEI_FRAMES {
                let Ok(d) = self.uread(c, buf, bytes as usize) else { return Some(Out::Ret(-EFAULT)) };
                data = d;
            }
        } else if inb > 0 {
            let Ok(d) = self.uread(c, arg, inb) else { return Some(Out::Ret(-EFAULT)) };
            data = d;
        }
        // a1: the argument itself (no data), or for ALSA's frames the frame
        // size; a2: bytes to read back, and frames (ALSA) above bit 32
        let xfer = cmd == SNDRV_PCM_IOCTL_WRITEI_FRAMES || cmd == SNDRV_PCM_IOCTL_READI_FRAMES;
        let a1 = if xfer {
            r.borrow().frame_bytes.max(1) as u64
        } else if inb == 0 && outb == 0 {
            arg
        } else {
            0
        };
        let (res, out) = match self.remote_call(c, sig, OP_IOCTL, h, [cmd, a1, (outb as u64) | (extra << 32)], &data) {
            None => return Some(Out::Block),
            Some(x) => x,
        };
        if cmd == FIONBIO && res == 0 {
            let on = data != [0, 0, 0, 0];
            let mut f = file.borrow_mut();
            f.flags = if on { f.flags | O_NONBLOCK } else { f.flags & !O_NONBLOCK };
        }
        if res < 0 {
            return Some(Out::Ret(res));
        }
        if cmd == SNDRV_PCM_IOCTL_HW_PARAMS && out.len() >= 276 {
            // intervals start at 260; FRAME_BITS is the second (min first)
            let bits = u32::from_le_bytes(out[272..276].try_into().unwrap());
            r.borrow_mut().frame_bytes = bits / 8;
        }
        if cmd == SNDRV_PCM_IOCTL_WRITEI_FRAMES || cmd == SNDRV_PCM_IOCTL_READI_FRAMES {
            // reply: i64 frames moved, then (reads) the frames
            if out.len() >= 8 {
                let _ = self.uwrite(c, arg, &out[0..8]);
                if cmd == SNDRV_PCM_IOCTL_READI_FRAMES {
                    if let Ok(buf) = self.uread_u64(c, arg + 8) {
                        let _ = self.uwrite(c, buf, &out[8..]);
                    }
                }
            }
            return Some(Out::Ret(res));
        }
        if outb > 0 && !out.is_empty() && self.uwrite(c, arg, &out[..out.len().min(outb)]).is_err() {
            return Some(Out::Ret(-EFAULT));
        }
        Some(Out::Ret(res))
    }
}

/// Readiness of a remote file for poll/epoll/select.
pub fn remote_poll(r: &RemoteRef) -> u32 {
    r.borrow().events
}
