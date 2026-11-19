//! Unix-domain sockets for the Linux personality: stream, datagram and
//! seqpacket; socketpair; named (file-system) and abstract addresses with
//! listen/accept/connect; descriptor passing (SCM_RIGHTS) and SO_PEERCRED.
//! Android's init, property service, logd and zygote all speak these.
//!
//! Internet sockets are not implemented yet (EAFNOSUPPORT).

use super::abi::*;
use super::fs::{open_file, FileKind, FileRef, EPOLLHUP, EPOLLIN, EPOLLOUT, EPOLLRDHUP};
use super::{Kernel, Out};
use alloc::collections::VecDeque;
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

const AF_UNIX: u64 = 1;
const SOCK_STREAM: u32 = 1;
const SOCK_DGRAM: u32 = 2;
const SOCK_SEQPACKET: u32 = 5;
const SOCK_NONBLOCK: u64 = 0o4000;
const SOCK_CLOEXEC: u64 = 0o2000000;
const SOL_SOCKET: u64 = 1;
const SCM_RIGHTS: i32 = 1;
const SO_TYPE: u64 = 3;
const SO_ERROR: u64 = 4;
const SO_SNDBUF: u64 = 7;
const SO_RCVBUF: u64 = 8;
const SO_PEERCRED: u64 = 17;
const SO_ACCEPTCONN: u64 = 30;
const MSG_TRUNC: u64 = 0x20;
const MSG_DONTWAIT: u64 = 0x40;
const MSG_NOSIGNAL: u64 = 0x4000;
const MSG_CMSG_CLOEXEC: u64 = 0x4000_0000;
const EADDRINUSE: i64 = 98;
const ECONNREFUSED: i64 = 111;
const ENOTCONN: i64 = 107;
const EISCONN: i64 = 106;
const ENOTSOCK: i64 = 88;
const EOPNOTSUPP: i64 = 95;

/// A received message: data, descriptors, truncated, sender name.
type Received = (Vec<u8>, Vec<FileRef>, bool, Option<String>);

pub struct Msg {
    data: Vec<u8>,
    fds: Vec<FileRef>,
    from: Option<String>,
}

#[derive(Clone, Copy, Default)]
pub struct Cred {
    pid: u32,
    uid: u32,
    gid: u32,
}

pub struct Socket {
    ty: u32,
    rx: VecDeque<Msg>,
    peer: Option<Weak<RefCell<Socket>>>,
    /// Pending connections, for a listening socket.
    queue: Option<VecDeque<Rc<RefCell<Socket>>>>,
    backlog: usize,
    name: Option<String>,
    peer_name: Option<String>,
    peer_cred: Cred,
    own_cred: Cred,
    shut_rd: bool,
    shut_wr: bool,
    /// Our peer shut down its writing side.
    peer_shut_wr: bool,
    was_connected: bool,
}

impl Socket {
    fn new(ty: u32, pid: u32) -> Socket {
        Socket {
            ty,
            rx: VecDeque::new(),
            peer: None,
            queue: None,
            backlog: 0,
            name: None,
            peer_name: None,
            peer_cred: Cred::default(),
            own_cred: Cred { pid, uid: 0, gid: 0 },
            shut_rd: false,
            shut_wr: false,
            peer_shut_wr: false,
            was_connected: false,
        }
    }

    fn peer(&self) -> Option<Rc<RefCell<Socket>>> {
        self.peer.as_ref().and_then(|p| p.upgrade())
    }

    fn eof(&self) -> bool {
        self.shut_rd || self.peer_shut_wr || (self.was_connected && self.peer().is_none())
    }

    pub fn close(&mut self) {
        if let Some(p) = self.peer() {
            if let Ok(mut p) = p.try_borrow_mut() {
                p.peer_shut_wr = true;
            }
        }
    }

    pub fn pending(&self) -> usize {
        self.rx.iter().map(|m| m.data.len()).sum()
    }

    pub fn poll(&self) -> u32 {
        if let Some(q) = &self.queue {
            return if q.is_empty() { 0 } else { EPOLLIN };
        }
        let mut ev = 0;
        if !self.rx.is_empty() || self.eof() {
            ev |= EPOLLIN;
        }
        if self.ty != SOCK_DGRAM && self.was_connected && self.peer().is_none() {
            ev |= EPOLLHUP | EPOLLRDHUP;
        } else if self.peer_shut_wr {
            ev |= EPOLLRDHUP;
        }
        if self.peer().is_some() || self.ty == SOCK_DGRAM {
            ev |= EPOLLOUT;
        }
        ev
    }

    /// Queue `data` (and descriptors) for our peer.
    pub fn send(&mut self, data: &[u8], fds: Vec<FileRef>, _pid: u32) -> Result<usize, i64> {
        if self.shut_wr {
            return Err(EPIPE);
        }
        let Some(peer) = self.peer() else {
            return Err(if self.was_connected { EPIPE } else { ENOTCONN });
        };
        let from = self.name.clone();
        let mut p = peer.borrow_mut();
        if p.shut_rd {
            return Err(EPIPE);
        }
        p.rx.push_back(Msg { data: data.to_vec(), fds, from });
        Ok(data.len())
    }

    /// Bytes for `read()`/`recv()`: `None` if the caller must wait.
    pub fn recv_bytes(&mut self, max: usize) -> Result<Option<Vec<u8>>, i64> {
        self.recv(max, false).map(|r| r.map(|(d, _, _, _)| d))
    }

    /// (data, descriptors, truncated, sender name)
    fn recv(&mut self, max: usize, peek: bool) -> Result<Option<Received>, i64> {
        if self.queue.is_some() {
            return Err(ENOTCONN);
        }
        if self.rx.is_empty() {
            return if self.eof() { Ok(Some((Vec::new(), Vec::new(), false, None))) } else { Ok(None) };
        }
        if self.ty == SOCK_STREAM {
            let mut out = Vec::new();
            let mut fds = Vec::new();
            while out.len() < max {
                let Some(m) = self.rx.front_mut() else { break };
                if !fds.is_empty() && !m.fds.is_empty() {
                    break; // descriptors stay with their own message
                }
                fds.append(&mut m.fds);
                let n = (max - out.len()).min(m.data.len());
                out.extend_from_slice(&m.data[..n]);
                if peek {
                    break;
                }
                m.data.drain(..n);
                if m.data.is_empty() {
                    self.rx.pop_front();
                }
            }
            return Ok(Some((out, fds, false, None)));
        }
        let m = if peek { None } else { self.rx.pop_front() };
        let m = match m {
            Some(m) => m,
            None => {
                let f = self.rx.front().unwrap();
                Msg { data: f.data.clone(), fds: Vec::new(), from: f.from.clone() }
            }
        };
        let trunc = m.data.len() > max;
        let n = m.data.len().min(max);
        Ok(Some((m.data[..n].to_vec(), m.fds, trunc, m.from)))
    }
}

fn parse_sockaddr(b: &[u8]) -> Result<String, i64> {
    if b.len() < 3 || u16::from_le_bytes([b[0], b[1]]) as u64 != AF_UNIX {
        return Err(EINVAL);
    }
    let path = &b[2..];
    if path[0] == 0 {
        // Abstract namespace: the name is the rest of the given length.
        return Ok(alloc::format!("@{}", String::from_utf8_lossy(&path[1..])));
    }
    let end = path.iter().position(|&x| x == 0).unwrap_or(path.len());
    Ok(String::from_utf8_lossy(&path[..end]).into_owned())
}

fn sockaddr_bytes(name: Option<&str>) -> Vec<u8> {
    let mut b = (AF_UNIX as u16).to_le_bytes().to_vec();
    if let Some(n) = name {
        if let Some(abs) = n.strip_prefix('@') {
            b.push(0);
            b.extend_from_slice(abs.as_bytes());
        } else {
            b.extend_from_slice(n.as_bytes());
            b.push(0);
        }
    }
    b
}

impl Kernel {
    fn sock(&mut self, c: usize, fd: u64) -> Result<(FileRef, Rc<RefCell<Socket>>), i64> {
        let f = self.proc(c).file(fd)?;
        let s = match &f.borrow().kind {
            FileKind::Socket(s) => s.clone(),
            _ => return Err(ENOTSOCK),
        };
        Ok((f, s))
    }

    fn install_sock(&mut self, c: usize, s: Socket, flags: u64) -> Result<usize, i64> {
        let f = open_file(
            FileKind::Socket(Rc::new(RefCell::new(s))),
            O_RDWR | if flags & SOCK_NONBLOCK != 0 { O_NONBLOCK } else { 0 },
            "socket:",
        );
        self.proc(c).install_fd(f, flags & SOCK_CLOEXEC != 0, 0)
    }

    pub(crate) fn sys_socket(&mut self, c: usize, domain: u64, ty: u64, _proto: u64) -> Result<i64, i64> {
        if domain != AF_UNIX {
            return Err(EAFNOSUPPORT);
        }
        let t = (ty & 0xf) as u32;
        if !matches!(t, SOCK_STREAM | SOCK_DGRAM | SOCK_SEQPACKET) {
            return Err(EINVAL);
        }
        let pid = self.proc(c).pid;
        self.install_sock(c, Socket::new(t, pid), ty).map(|n| n as i64)
    }

    pub(crate) fn sys_socketpair(&mut self, c: usize, domain: u64, ty: u64, _proto: u64, sv: u64) -> Result<i64, i64> {
        if domain != AF_UNIX {
            return Err(EAFNOSUPPORT);
        }
        let t = (ty & 0xf) as u32;
        let pid = self.proc(c).pid;
        let a = Rc::new(RefCell::new(Socket::new(t, pid)));
        let b = Rc::new(RefCell::new(Socket::new(t, pid)));
        for (x, y) in [(&a, &b), (&b, &a)] {
            let mut x = x.borrow_mut();
            x.peer = Some(Rc::downgrade(y));
            x.was_connected = true;
            x.peer_cred = Cred { pid, uid: 0, gid: 0 };
        }
        let fl = O_RDWR | if ty & SOCK_NONBLOCK != 0 { O_NONBLOCK } else { 0 };
        let fa = open_file(FileKind::Socket(a), fl, "socket:");
        let fb = open_file(FileKind::Socket(b), fl, "socket:");
        let pair = self.install_pair(c, fa, fb, ty & SOCK_CLOEXEC != 0)?;
        self.uwrite(c, sv, &pair).map(|_| 0)
    }

    pub(crate) fn sys_bind(&mut self, c: usize, fd: u64, addr: u64, len: u64) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        let raw = self.uread(c, addr, (len as usize).min(110))?;
        let name = parse_sockaddr(&raw)?;
        let lx = self.lx();
        if lx.unix_names.get(&name).is_some_and(|w| w.strong_count() > 0) {
            return Err(EADDRINUSE);
        }
        if !name.starts_with('@') {
            let cwd = self.proc(c).cwd.clone();
            let lx = self.lx();
            let r = lx.vfs.create(&cwd, &name, 0o777, true).map_err(|e| if e == EEXIST { EADDRINUSE } else { e })?;
            r.node.borrow_mut().mode = S_IFSOCK | 0o777;
        }
        let key = if name.starts_with('@') { name.clone() } else { super::vfs::absolute(&self.proc(c).cwd, &name) };
        self.lx().unix_names.insert(key.clone(), Rc::downgrade(&s));
        s.borrow_mut().name = Some(if name.starts_with('@') { name } else { key });
        Ok(0)
    }

    pub(crate) fn sys_listen(&mut self, c: usize, fd: u64, backlog: u64) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        let mut s = s.borrow_mut();
        if s.ty == SOCK_DGRAM {
            return Err(EOPNOTSUPP);
        }
        if s.queue.is_none() {
            s.queue = Some(VecDeque::new());
        }
        s.backlog = (backlog as usize).clamp(1, 128);
        Ok(0)
    }

    fn lookup_name(&mut self, c: usize, raw: &[u8]) -> Result<Rc<RefCell<Socket>>, i64> {
        let name = parse_sockaddr(raw)?;
        let key = if name.starts_with('@') { name } else { super::vfs::absolute(&self.proc(c).cwd, &name) };
        self.lx().unix_names.get(&key).and_then(|w| w.upgrade()).ok_or(ECONNREFUSED)
    }

    pub(crate) fn sys_connect(&mut self, c: usize, fd: u64, addr: u64, len: u64) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        let raw = self.uread(c, addr, (len as usize).min(110))?;
        let target = self.lookup_name(c, &raw)?;
        let pid = self.proc(c).pid;
        let ty = s.borrow().ty;
        if ty == SOCK_DGRAM {
            let mut sb = s.borrow_mut();
            sb.peer = Some(Rc::downgrade(&target));
            sb.peer_name = target.borrow().name.clone();
            return Ok(0);
        }
        if s.borrow().peer().is_some() {
            return Err(EISCONN);
        }
        let mut l = target.borrow_mut();
        let (listener_cred, lname, backlog) = (l.own_cred, l.name.clone(), l.backlog);
        let Some(q) = l.queue.as_mut() else { return Err(ECONNREFUSED) };
        if q.len() >= backlog {
            return Err(EAGAIN);
        }
        let server = Rc::new(RefCell::new(Socket::new(ty, listener_cred.pid)));
        {
            let mut sv = server.borrow_mut();
            sv.peer = Some(Rc::downgrade(&s));
            sv.was_connected = true;
            sv.peer_cred = Cred { pid, uid: 0, gid: 0 };
            sv.name = lname.clone();
        }
        q.push_back(server.clone());
        let mut sb = s.borrow_mut();
        sb.peer = Some(Rc::downgrade(&server));
        sb.was_connected = true;
        sb.peer_cred = listener_cred;
        sb.peer_name = lname;
        // The pending server socket is owned by the queue until accepted.
        Ok(0)
    }

    pub(crate) fn sys_accept4(&mut self, c: usize, fd: u64, addr: u64, alen: u64, flags: u64) -> Out {
        let (f, s) = match self.sock(c, fd) {
            Ok(x) => x,
            Err(e) => return Out::Ret(-e),
        };
        let nonblock = f.borrow().flags & O_NONBLOCK != 0;
        let next = {
            let mut sb = s.borrow_mut();
            match sb.queue.as_mut() {
                None => return Out::Ret(-EINVAL),
                Some(q) => q.pop_front(),
            }
        };
        let Some(conn) = next else {
            return if nonblock { Out::Ret(-EAGAIN) } else { Out::Block };
        };
        let peer_name = conn.borrow().peer().and_then(|p| p.borrow().name.clone());
        let fl = O_RDWR | if flags & SOCK_NONBLOCK != 0 { O_NONBLOCK } else { 0 };
        let file = open_file(FileKind::Socket(conn), fl, "socket:");
        let n = match self.proc(c).install_fd(file, flags & SOCK_CLOEXEC != 0, 0) {
            Ok(n) => n,
            Err(e) => return Out::Ret(-e),
        };
        if addr != 0 && alen != 0 {
            let b = sockaddr_bytes(peer_name.as_deref());
            let _ = self.uwrite(c, addr, &b);
            let _ = self.uwrite(c, alen, &(b.len() as u32).to_le_bytes());
        }
        Out::Ret(n as i64)
    }

    pub(crate) fn sys_getsockname(&mut self, c: usize, fd: u64, addr: u64, alen: u64, peer: bool) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        let name = {
            let s = s.borrow();
            if peer {
                if s.peer().is_none() {
                    return Err(ENOTCONN);
                }
                s.peer_name.clone()
            } else {
                s.name.clone()
            }
        };
        let b = sockaddr_bytes(name.as_deref());
        self.uwrite(c, addr, &b)?;
        self.uwrite(c, alen, &(b.len() as u32).to_le_bytes()).map(|_| 0)
    }

    pub(crate) fn sys_getsockopt(
        &mut self,
        c: usize,
        fd: u64,
        level: u64,
        opt: u64,
        val: u64,
        len: u64,
    ) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        if level != SOL_SOCKET {
            return Err(EOPNOTSUPP);
        }
        let v: Vec<u8> = {
            let s = s.borrow();
            match opt {
                SO_PEERCRED => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&s.peer_cred.pid.to_le_bytes());
                    b.extend_from_slice(&s.peer_cred.uid.to_le_bytes());
                    b.extend_from_slice(&s.peer_cred.gid.to_le_bytes());
                    b
                }
                SO_TYPE => s.ty.to_le_bytes().to_vec(),
                SO_ERROR => 0u32.to_le_bytes().to_vec(),
                SO_SNDBUF | SO_RCVBUF => 212_992u32.to_le_bytes().to_vec(),
                SO_ACCEPTCONN => (s.queue.is_some() as u32).to_le_bytes().to_vec(),
                _ => 0u32.to_le_bytes().to_vec(),
            }
        };
        self.uwrite(c, val, &v)?;
        self.uwrite(c, len, &(v.len() as u32).to_le_bytes()).map(|_| 0)
    }

    pub(crate) fn sys_shutdown(&mut self, c: usize, fd: u64, how: u64) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        let mut sb = s.borrow_mut();
        if how == 0 || how == 2 {
            sb.shut_rd = true;
        }
        if how == 1 || how == 2 {
            sb.shut_wr = true;
            if let Some(p) = sb.peer() {
                p.borrow_mut().peer_shut_wr = true;
            }
        }
        Ok(0)
    }

    /// sendto / sendmsg. `iov` is a list of (base, len).
    #[allow(clippy::too_many_arguments)]
    fn sock_send(
        &mut self,
        c: usize,
        fd: u64,
        data: Vec<u8>,
        fds: Vec<FileRef>,
        flags: u64,
        dest: Option<Vec<u8>>,
    ) -> Result<i64, i64> {
        let (_, s) = self.sock(c, fd)?;
        let pid = self.proc(c).pid;
        let r = match dest {
            Some(raw) if s.borrow().ty == SOCK_DGRAM => {
                let target = self.lookup_name(c, &raw)?;
                let from = s.borrow().name.clone();
                target.borrow_mut().rx.push_back(Msg { data: data.clone(), fds, from });
                Ok(data.len())
            }
            _ => s.borrow_mut().send(&data, fds, pid),
        };
        if r == Err(EPIPE) && flags & MSG_NOSIGNAL == 0 {
            self.sig_send_thread(c, super::signal::SigInfo::user(SIGPIPE, pid));
        }
        r.map(|n| n as i64)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sys_sendto(
        &mut self,
        c: usize,
        fd: u64,
        buf: u64,
        len: u64,
        flags: u64,
        addr: u64,
        alen: u64,
    ) -> Result<i64, i64> {
        let data = self.uread(c, buf, (len as usize).min(1 << 20))?;
        let dest = if addr != 0 { Some(self.uread(c, addr, (alen as usize).min(110))?) } else { None };
        self.sock_send(c, fd, data, Vec::new(), flags, dest)
    }

    pub(crate) fn sys_sendmsg(&mut self, c: usize, fd: u64, msg: u64, flags: u64) -> Result<i64, i64> {
        let h = self.uread(c, msg, 56)?;
        let g = |o: usize| u64::from_le_bytes(h[o..o + 8].try_into().unwrap());
        let (name, namelen, iov, iovlen, ctl, ctllen) = (g(0), g(8) & 0xffff_ffff, g(16), g(24), g(32), g(40));
        let mut data = Vec::new();
        for i in 0..iovlen.min(1024) {
            let base = self.uread_u64(c, iov + i * 16)?;
            let len = self.uread_u64(c, iov + i * 16 + 8)?;
            data.extend(self.uread(c, base, (len as usize).min(1 << 20))?);
        }
        let mut fds = Vec::new();
        if ctl != 0 && ctllen >= 16 {
            let cb = self.uread(c, ctl, (ctllen as usize).min(4096))?;
            let mut off = 0usize;
            while off + 16 <= cb.len() {
                let clen = u64::from_le_bytes(cb[off..off + 8].try_into().unwrap()) as usize;
                let level = i32::from_le_bytes(cb[off + 8..off + 12].try_into().unwrap());
                let ty = i32::from_le_bytes(cb[off + 12..off + 16].try_into().unwrap());
                if clen < 16 || off + clen > cb.len() {
                    break;
                }
                if level == SOL_SOCKET as i32 && ty == SCM_RIGHTS {
                    for chunk in cb[off + 16..off + clen].chunks_exact(4) {
                        let n = i32::from_le_bytes(chunk.try_into().unwrap());
                        fds.push(self.proc(c).file(n as u64)?);
                    }
                }
                off += clen.div_ceil(8) * 8;
            }
        }
        let dest =
            if name != 0 && namelen != 0 { Some(self.uread(c, name, (namelen as usize).min(110))?) } else { None };
        self.sock_send(c, fd, data, fds, flags, dest)
    }

    fn sock_recv(&mut self, c: usize, fd: u64, max: usize, flags: u64) -> Result<Option<Received>, i64> {
        let (f, s) = self.sock(c, fd)?;
        let nonblock = f.borrow().flags & O_NONBLOCK != 0 || flags & MSG_DONTWAIT != 0;
        let r = s.borrow_mut().recv(max, flags & 2 != 0)?;
        match r {
            None if nonblock => Err(EAGAIN),
            other => Ok(other),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sys_recvfrom(
        &mut self,
        c: usize,
        fd: u64,
        buf: u64,
        len: u64,
        flags: u64,
        addr: u64,
        alen: u64,
    ) -> Out {
        match self.sock_recv(c, fd, (len as usize).min(1 << 20), flags) {
            Err(e) => Out::Ret(-e),
            Ok(None) => Out::Block,
            Ok(Some((data, _fds, trunc, from))) => {
                if let Err(e) = self.uwrite(c, buf, &data) {
                    return Out::Ret(-e);
                }
                if addr != 0 && alen != 0 {
                    let b = sockaddr_bytes(from.as_deref());
                    let _ = self.uwrite(c, addr, &b);
                    let _ = self.uwrite(c, alen, &(b.len() as u32).to_le_bytes());
                }
                let _ = trunc;
                Out::Ret(data.len() as i64)
            }
        }
    }

    pub(crate) fn sys_recvmsg(&mut self, c: usize, fd: u64, msg: u64, flags: u64) -> Out {
        let h = match self.uread(c, msg, 56) {
            Ok(h) => h,
            Err(e) => return Out::Ret(-e),
        };
        let g = |o: usize| u64::from_le_bytes(h[o..o + 8].try_into().unwrap());
        let (name, iov, iovlen, ctl, ctllen) = (g(0), g(16), g(24), g(32), g(40));
        let mut iovs = Vec::new();
        let mut total = 0usize;
        for i in 0..iovlen.min(1024) {
            match (self.uread_u64(c, iov + i * 16), self.uread_u64(c, iov + i * 16 + 8)) {
                (Ok(b), Ok(l)) => {
                    iovs.push((b, l as usize));
                    total += l as usize;
                }
                _ => return Out::Ret(-EFAULT),
            }
        }
        let (data, fds, trunc, from) = match self.sock_recv(c, fd, total.min(1 << 20), flags) {
            Err(e) => return Out::Ret(-e),
            Ok(None) => return Out::Block,
            Ok(Some(x)) => x,
        };
        let mut off = 0;
        for (b, l) in iovs {
            if off >= data.len() {
                break;
            }
            let n = l.min(data.len() - off);
            if let Err(e) = self.uwrite(c, b, &data[off..off + n]) {
                return Out::Ret(-e);
            }
            off += n;
        }
        let mut out_flags = if trunc { MSG_TRUNC as u32 } else { 0 };
        let mut ctl_used = 0u64;
        if !fds.is_empty() {
            let cloexec = flags & MSG_CMSG_CLOEXEC != 0;
            let mut nums = Vec::new();
            for f in fds {
                match self.proc(c).install_fd(f, cloexec, 0) {
                    Ok(n) => nums.push(n as i32),
                    Err(_) => break,
                }
            }
            let clen = 16 + nums.len() * 4;
            if ctl != 0 && ctllen as usize >= clen {
                let mut cb = Vec::new();
                cb.extend_from_slice(&(clen as u64).to_le_bytes());
                cb.extend_from_slice(&(SOL_SOCKET as i32).to_le_bytes());
                cb.extend_from_slice(&SCM_RIGHTS.to_le_bytes());
                for n in &nums {
                    cb.extend_from_slice(&n.to_le_bytes());
                }
                if let Err(e) = self.uwrite(c, ctl, &cb) {
                    return Out::Ret(-e);
                }
                ctl_used = clen.div_ceil(8) as u64 * 8;
            } else {
                out_flags |= 0x8; // MSG_CTRUNC
            }
        }
        let _ = self.uwrite(c, msg + 40, &ctl_used.to_le_bytes());
        let _ = self.uwrite(c, msg + 48, &out_flags.to_le_bytes());
        if name != 0 {
            let b = sockaddr_bytes(from.as_deref());
            let _ = self.uwrite(c, name, &b);
            let _ = self.uwrite(c, msg + 8, &(b.len() as u32).to_le_bytes());
        }
        Out::Ret(data.len() as i64)
    }
}
