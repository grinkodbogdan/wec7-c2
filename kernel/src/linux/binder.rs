//! Binder, Android's IPC, as the Linux binder driver presents it
//! (`/dev/binder`, `/dev/hwbinder`, `/dev/vndbinder`; protocol version 8,
//! 64-bit structures), so libbinder and servicemanager can run unmodified.
//!
//! Each open of a binder device is a [`BinderProc`]. A process `mmap`s a
//! receive area; transactions sent to it are copied into that area (backed
//! by a shared memory object, so the kernel writes it directly) and handed
//! to one of its threads as `BR_TRANSACTION`. Binder objects in a
//! transaction are translated: a local object becomes a handle in the
//! receiver, a handle to the receiver's own object becomes the object again,
//! and file descriptors are duplicated into the receiver.
//!
//! Not yet: scatter-gather (`BC_TRANSACTION_SG`, used by HIDL), strong/weak
//! reference bookkeeping towards node owners (`BR_INCREFS`/`BR_ACQUIRE`),
//! thread-pool spawning hints, priority inheritance.

use super::abi::*;
use super::fs::{FileKind, FileRef};
use super::mm::{MemObj, MemRef};
use super::{Kernel, Out, Process};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

pub const BINDER_WRITE_READ: u64 = 0xc030_6201;
pub const BINDER_SET_MAX_THREADS: u64 = 0x4004_6205;
pub const BINDER_SET_CONTEXT_MGR: u64 = 0x4004_6207;
pub const BINDER_THREAD_EXIT: u64 = 0x4004_6208;
pub const BINDER_VERSION: u64 = 0xc004_6209;
pub const BINDER_SET_CONTEXT_MGR_EXT: u64 = 0x4018_620d;
pub const BINDER_ENABLE_ONEWAY_SPAM_DETECTION: u64 = 0x4004_6210;
const PROTOCOL_VERSION: i32 = 8;

const BC_TRANSACTION: u32 = 0x4040_6300;
const BC_REPLY: u32 = 0x4040_6301;
const BC_FREE_BUFFER: u32 = 0x4008_6303;
const BC_INCREFS: u32 = 0x4004_6304;
const BC_ACQUIRE: u32 = 0x4004_6305;
const BC_RELEASE: u32 = 0x4004_6306;
const BC_DECREFS: u32 = 0x4004_6307;
const BC_INCREFS_DONE: u32 = 0x4010_6308;
const BC_ACQUIRE_DONE: u32 = 0x4010_6309;
const BC_REGISTER_LOOPER: u32 = 0x630b;
const BC_ENTER_LOOPER: u32 = 0x630c;
const BC_EXIT_LOOPER: u32 = 0x630d;
const BC_REQUEST_DEATH_NOTIFICATION: u32 = 0x400c_630e;
const BC_CLEAR_DEATH_NOTIFICATION: u32 = 0x400c_630f;
const BC_DEAD_BINDER_DONE: u32 = 0x4008_6310;

const BR_TRANSACTION: u32 = 0x8040_7202;
const BR_REPLY: u32 = 0x8040_7203;
const BR_DEAD_REPLY: u32 = 0x7205;
const BR_TRANSACTION_COMPLETE: u32 = 0x7206;
const BR_NOOP: u32 = 0x720c;
const BR_DEAD_BINDER: u32 = 0x8008_720f;
const BR_CLEAR_DEATH_NOTIFICATION_DONE: u32 = 0x8008_7210;
const BR_FAILED_REPLY: u32 = 0x7211;

const TF_ONE_WAY: u32 = 0x01;
const TF_STATUS_CODE: u32 = 0x08;

const TYPE_BINDER: u32 = 0x7362_2a85;
const TYPE_WEAK_BINDER: u32 = 0x7762_2a85;
const TYPE_HANDLE: u32 = 0x7368_2a85;
const TYPE_WEAK_HANDLE: u32 = 0x7768_2a85;
const TYPE_FD: u32 = 0x6664_2a85;

/// A binder object living in some process.
pub struct Node {
    ptr: u64,
    cookie: u64,
    owner: Weak<RefCell<BinderProc>>,
    owner_pid: u32,
    /// Processes that asked to hear when this node dies, with their cookie.
    watchers: Vec<(Weak<RefCell<BinderProc>>, u64)>,
}

pub type NodeRef = Rc<RefCell<Node>>;
type ProcRef = Rc<RefCell<BinderProc>>;
/// A thread waiting for a reply: its binder proc and tid.
type Caller = (Weak<RefCell<BinderProc>>, u32);

#[derive(Clone, Copy, Default)]
struct TxnData {
    target: u64,
    cookie: u64,
    code: u32,
    flags: u32,
    sender_pid: u32,
    data_size: u64,
    offsets_size: u64,
    buffer: u64,
    offsets: u64,
}

impl TxnData {
    fn to_bytes(self) -> [u8; 64] {
        let mut b = [0u8; 64];
        b[0..8].copy_from_slice(&self.target.to_le_bytes());
        b[8..16].copy_from_slice(&self.cookie.to_le_bytes());
        b[16..20].copy_from_slice(&self.code.to_le_bytes());
        b[20..24].copy_from_slice(&self.flags.to_le_bytes());
        b[24..28].copy_from_slice(&self.sender_pid.to_le_bytes());
        b[32..40].copy_from_slice(&self.data_size.to_le_bytes());
        b[40..48].copy_from_slice(&self.offsets_size.to_le_bytes());
        b[48..56].copy_from_slice(&self.buffer.to_le_bytes());
        b[56..64].copy_from_slice(&self.offsets.to_le_bytes());
        b
    }
}

enum Work {
    Transaction { tr: TxnData, reply_to: Option<Caller> },
    Reply(TxnData),
    Complete,
    DeadReply,
    FailedReply,
    DeadBinder(u64),
    ClearDeathDone(u64),
}

/// One open of a binder device (in practice: one per process).
pub struct BinderProc {
    pub pid: u32,
    device: String,
    process: Weak<RefCell<Process>>,
    buf: Option<(MemRef, u64, u64)>,
    allocs: Vec<(u64, u64)>,
    nodes: BTreeMap<u64, NodeRef>,
    refs: BTreeMap<u32, NodeRef>,
    next_handle: u32,
    todo: VecDeque<Work>,
    thread_todo: BTreeMap<u32, VecDeque<Work>>,
    /// For each thread: the callers it owes replies to (innermost last).
    stacks: BTreeMap<u32, Vec<Caller>>,
    /// Transactions queued on `todo` whose senders wait for a reply.
    waiting: Vec<Caller>,
}

impl BinderProc {
    pub fn new(pid: u32, device: &str, process: Weak<RefCell<Process>>) -> BinderProc {
        BinderProc {
            pid,
            device: String::from(device),
            process,
            buf: None,
            allocs: Vec::new(),
            nodes: BTreeMap::new(),
            refs: BTreeMap::new(),
            next_handle: 1,
            todo: VecDeque::new(),
            thread_todo: BTreeMap::new(),
            stacks: BTreeMap::new(),
            waiting: Vec::new(),
        }
    }

    /// The receive area, created by `mmap` of the binder fd.
    pub fn map(&mut self, va: u64, size: u64) -> MemRef {
        let obj = Rc::new(RefCell::new(MemObj::new(size)));
        self.buf = Some((obj.clone(), va, size));
        obj
    }

    fn alloc(&mut self, size: u64) -> Option<u64> {
        let (_, _, total) = self.buf.as_ref()?;
        let size = size.max(8).div_ceil(8) * 8;
        let mut start = 0;
        for &(s, l) in &self.allocs {
            if start + size <= s {
                break;
            }
            start = s + l;
        }
        if start + size > *total {
            return None;
        }
        let i = self.allocs.iter().position(|&(s, _)| s > start).unwrap_or(self.allocs.len());
        self.allocs.insert(i, (start, size));
        Some(start)
    }

    fn free(&mut self, va: u64) {
        if let Some((_, base, _)) = self.buf {
            let off = va.wrapping_sub(base);
            self.allocs.retain(|&(s, _)| s != off);
        }
    }

    fn handle_for(&mut self, node: &NodeRef, ctx: Option<&NodeRef>) -> u32 {
        if ctx.is_some_and(|c| Rc::ptr_eq(c, node)) {
            return 0;
        }
        if let Some((&h, _)) = self.refs.iter().find(|(_, n)| Rc::ptr_eq(n, node)) {
            return h;
        }
        let h = self.next_handle;
        self.next_handle += 1;
        self.refs.insert(h, node.clone());
        h
    }

    pub fn readable(&self, tid: u32) -> bool {
        !self.todo.is_empty() || self.thread_todo.get(&tid).is_some_and(|q| !q.is_empty())
    }
}

impl Drop for BinderProc {
    /// The process is gone: its objects die, callers waiting on it get a
    /// dead reply.
    fn drop(&mut self) {
        for node in self.nodes.values() {
            for (w, cookie) in core::mem::take(&mut node.borrow_mut().watchers) {
                if let Some(p) = w.upgrade() {
                    if let Ok(mut p) = p.try_borrow_mut() {
                        p.todo.push_back(Work::DeadBinder(cookie));
                    }
                }
            }
        }
        let callers: Vec<_> = self.stacks.values().flatten().cloned().chain(self.waiting.drain(..)).collect();
        for (w, tid) in callers {
            if let Some(p) = w.upgrade() {
                if let Ok(mut p) = p.try_borrow_mut() {
                    p.thread_todo.entry(tid).or_default().push_back(Work::DeadReply);
                }
            }
        }
    }
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

impl Kernel {
    fn binder_context(&mut self, device: &str) -> Option<NodeRef> {
        let ctx = self.lx().binder_ctx.get(device).cloned()?;
        if ctx.borrow().owner.strong_count() == 0 {
            self.lx().binder_ctx.remove(device);
            return None;
        }
        Some(ctx)
    }

    pub(crate) fn binder_open(&mut self, c: usize, device: &str) -> ProcRef {
        let pid = self.proc(c).pid;
        let process = Rc::downgrade(&self.comps[c].linux.as_ref().unwrap().p);
        Rc::new(RefCell::new(BinderProc::new(pid, device, process)))
    }

    pub(crate) fn binder_ioctl(&mut self, c: usize, b: ProcRef, nonblock: bool, req: u64, arg: u64) -> Out {
        let r = match req {
            BINDER_VERSION => self.uwrite(c, arg, &PROTOCOL_VERSION.to_le_bytes()).map(|_| 0),
            BINDER_SET_MAX_THREADS | BINDER_THREAD_EXIT | BINDER_ENABLE_ONEWAY_SPAM_DETECTION => Ok(0),
            BINDER_SET_CONTEXT_MGR | BINDER_SET_CONTEXT_MGR_EXT => {
                let (ptr, cookie) = if req == BINDER_SET_CONTEXT_MGR_EXT {
                    match self.uread(c, arg, 24) {
                        Ok(o) => (u64_at(&o, 8), u64_at(&o, 16)),
                        Err(e) => return Out::Ret(-e),
                    }
                } else {
                    (0, 0)
                };
                let device = b.borrow().device.clone();
                if self.binder_context(&device).is_some() {
                    Err(EBUSY)
                } else {
                    let pid = b.borrow().pid;
                    let node = Rc::new(RefCell::new(Node {
                        ptr,
                        cookie,
                        owner: Rc::downgrade(&b),
                        owner_pid: pid,
                        watchers: Vec::new(),
                    }));
                    b.borrow_mut().nodes.insert(ptr, node.clone());
                    self.lx().binder_ctx.insert(device, node);
                    Ok(0)
                }
            }
            BINDER_WRITE_READ => return self.binder_write_read(c, &b, nonblock, arg),
            _ => Err(EINVAL),
        };
        match r {
            Ok(v) => Out::Ret(v),
            Err(e) => Out::Ret(-e),
        }
    }

    fn binder_write_read(&mut self, c: usize, b: &ProcRef, nonblock: bool, arg: u64) -> Out {
        let hdr = match self.uread(c, arg, 48) {
            Ok(h) => h,
            Err(e) => return Out::Ret(-e),
        };
        let (wsize, mut wcons, wbuf) = (u64_at(&hdr, 0), u64_at(&hdr, 8), u64_at(&hdr, 16));
        let (rsize, mut rcons, rbuf) = (u64_at(&hdr, 24), u64_at(&hdr, 32), u64_at(&hdr, 40));
        let tid = self.task(c).tid;
        // Commands are processed once: write_consumed is stored before any wait.
        if wcons < wsize {
            let cmds = match self.uread(c, wbuf + wcons, (wsize - wcons) as usize) {
                Ok(v) => v,
                Err(e) => return Out::Ret(-e),
            };
            let mut off = 0usize;
            while off + 4 <= cmds.len() {
                let cmd = u32_at(&cmds, off);
                let size = ((cmd >> 16) & 0x3fff) as usize;
                if off + 4 + size > cmds.len() {
                    break;
                }
                let payload = cmds[off + 4..off + 4 + size].to_vec();
                off += 4 + size;
                if let Err(e) = self.binder_command(c, b, tid, cmd, &payload) {
                    wcons += off as u64;
                    let _ = self.uwrite(c, arg + 8, &wcons.to_le_bytes());
                    return Out::Ret(-e);
                }
            }
            wcons += off as u64;
            if let Err(e) = self.uwrite(c, arg + 8, &wcons.to_le_bytes()) {
                return Out::Ret(-e);
            }
        }
        if rsize == 0 || rcons >= rsize {
            return Out::Ret(0);
        }
        // Read: thread work first, then process work.
        let mut out: Vec<u8> = Vec::new();
        if rcons == 0 {
            out.extend_from_slice(&BR_NOOP.to_le_bytes());
        }
        let room = (rsize - rcons) as usize;
        loop {
            let work = {
                let mut bp = b.borrow_mut();
                let next_len = |w: &Work| match w {
                    Work::Transaction { .. } | Work::Reply(_) => 68,
                    Work::DeadBinder(_) | Work::ClearDeathDone(_) => 12,
                    _ => 4,
                };
                let fits = |q: &VecDeque<Work>| q.front().is_some_and(|w| out.len() + next_len(w) <= room);
                if bp.thread_todo.get(&tid).is_some_and(fits) {
                    bp.thread_todo.get_mut(&tid).unwrap().pop_front()
                } else if fits(&bp.todo) {
                    bp.todo.pop_front()
                } else {
                    None
                }
            };
            let Some(work) = work else { break };
            match work {
                Work::Transaction { tr, reply_to } => {
                    out.extend_from_slice(&BR_TRANSACTION.to_le_bytes());
                    out.extend_from_slice(&tr.to_bytes());
                    if let Some(rt) = reply_to {
                        let mut bp = b.borrow_mut();
                        bp.waiting.retain(|(w, t)| !(w.ptr_eq(&rt.0) && *t == rt.1));
                        bp.stacks.entry(tid).or_default().push(rt);
                    }
                    break; // one transaction per read, like the driver
                }
                Work::Reply(tr) => {
                    out.extend_from_slice(&BR_REPLY.to_le_bytes());
                    out.extend_from_slice(&tr.to_bytes());
                    break;
                }
                Work::Complete => out.extend_from_slice(&BR_TRANSACTION_COMPLETE.to_le_bytes()),
                Work::DeadReply => {
                    out.extend_from_slice(&BR_DEAD_REPLY.to_le_bytes());
                    break;
                }
                Work::FailedReply => {
                    out.extend_from_slice(&BR_FAILED_REPLY.to_le_bytes());
                    break;
                }
                Work::DeadBinder(cookie) => {
                    out.extend_from_slice(&BR_DEAD_BINDER.to_le_bytes());
                    out.extend_from_slice(&cookie.to_le_bytes());
                }
                Work::ClearDeathDone(cookie) => {
                    out.extend_from_slice(&BR_CLEAR_DEATH_NOTIFICATION_DONE.to_le_bytes());
                    out.extend_from_slice(&cookie.to_le_bytes());
                }
            }
        }
        let got_work = out.len() > if rcons == 0 { 4 } else { 0 };
        if !got_work && !nonblock {
            return Out::Block;
        }
        if let Err(e) = self.uwrite(c, rbuf + rcons, &out) {
            return Out::Ret(-e);
        }
        rcons += out.len() as u64;
        match self.uwrite(c, arg + 32, &rcons.to_le_bytes()) {
            Ok(()) if !got_work => Out::Ret(-EAGAIN),
            Ok(()) => Out::Ret(0),
            Err(e) => Out::Ret(-e),
        }
    }

    fn binder_command(&mut self, c: usize, b: &ProcRef, tid: u32, cmd: u32, p: &[u8]) -> Result<(), i64> {
        match cmd {
            BC_TRANSACTION | BC_REPLY => self.binder_transact(c, b, tid, cmd == BC_REPLY, p),
            BC_FREE_BUFFER => {
                b.borrow_mut().free(u64_at(p, 0));
                Ok(())
            }
            BC_INCREFS | BC_ACQUIRE | BC_RELEASE | BC_DECREFS | BC_INCREFS_DONE | BC_ACQUIRE_DONE
            | BC_REGISTER_LOOPER | BC_ENTER_LOOPER | BC_EXIT_LOOPER | BC_DEAD_BINDER_DONE => Ok(()),
            BC_REQUEST_DEATH_NOTIFICATION | BC_CLEAR_DEATH_NOTIFICATION => {
                let (handle, cookie) = (u32_at(p, 0), u64_at(p, 4));
                let device = b.borrow().device.clone();
                let node =
                    if handle == 0 { self.binder_context(&device) } else { b.borrow().refs.get(&handle).cloned() };
                let mut bp = b.borrow_mut();
                let Some(node) = node else {
                    if cmd == BC_REQUEST_DEATH_NOTIFICATION {
                        bp.todo.push_back(Work::DeadBinder(cookie));
                    }
                    return Ok(());
                };
                let mut n = node.borrow_mut();
                if cmd == BC_CLEAR_DEATH_NOTIFICATION {
                    n.watchers.retain(|(w, ck)| !(w.ptr_eq(&Rc::downgrade(b)) && *ck == cookie));
                    bp.thread_todo.entry(tid).or_default().push_back(Work::ClearDeathDone(cookie));
                } else if n.owner.strong_count() == 0 {
                    bp.todo.push_back(Work::DeadBinder(cookie));
                } else {
                    n.watchers.push((Rc::downgrade(b), cookie));
                }
                Ok(())
            }
            _ => {
                kprintln!("linux: binder: unsupported command {:#x}", cmd);
                Err(EINVAL)
            }
        }
    }

    /// BC_TRANSACTION / BC_REPLY: copy the payload into the receiver's area,
    /// translate the objects in it and queue it.
    fn binder_transact(&mut self, c: usize, b: &ProcRef, tid: u32, reply: bool, p: &[u8]) -> Result<(), i64> {
        let handle = u32_at(p, 0);
        let code = u32_at(p, 16);
        let flags = u32_at(p, 20);
        let (data_size, offsets_size) = (u64_at(p, 32), u64_at(p, 40));
        let (data_ptr, offsets_ptr) = (u64_at(p, 48), u64_at(p, 56));
        let device = b.borrow().device.clone();
        let ctx = self.binder_context(&device);
        let fail = |b: &ProcRef, w: Work| {
            let mut bp = b.borrow_mut();
            let q = bp.thread_todo.entry(tid).or_default();
            q.push_back(Work::Complete);
            q.push_back(w);
        };

        // Who receives it.
        let (target, node, reply_to): (ProcRef, Option<NodeRef>, Option<Caller>) = if reply {
            let Some((w, caller_tid)) = b.borrow_mut().stacks.get_mut(&tid).and_then(|s| s.pop()) else {
                return Err(EINVAL);
            };
            match w.upgrade() {
                Some(t) => (t, None, Some((w, caller_tid))),
                None => {
                    b.borrow_mut().thread_todo.entry(tid).or_default().push_back(Work::Complete);
                    return Ok(());
                }
            }
        } else {
            let node = if handle == 0 { ctx.clone() } else { b.borrow().refs.get(&handle).cloned() };
            let Some(node) = node else {
                fail(b, Work::DeadReply);
                return Ok(());
            };
            let owner = node.borrow().owner.upgrade();
            let Some(owner) = owner else {
                fail(b, Work::DeadReply);
                return Ok(());
            };
            (owner, Some(node), None)
        };

        let mut data = self.uread(c, data_ptr, data_size.min(1 << 20) as usize)?;
        let offsets = self.uread(c, offsets_ptr, offsets_size.min(1 << 16) as usize)?;
        let same = Rc::ptr_eq(&target, b);
        // Translate objects.
        for chunk in offsets.chunks_exact(8) {
            let off = u64::from_le_bytes(chunk.try_into().unwrap()) as usize;
            if off + 24 > data.len() {
                return Err(EINVAL);
            }
            let ty = u32_at(&data, off);
            match ty {
                TYPE_BINDER | TYPE_WEAK_BINDER => {
                    let (ptr, cookie) = (u64_at(&data, off + 8), u64_at(&data, off + 16));
                    let node = {
                        let mut bp = b.borrow_mut();
                        let pid = bp.pid;
                        bp.nodes
                            .entry(ptr)
                            .or_insert_with(|| {
                                Rc::new(RefCell::new(Node {
                                    ptr,
                                    cookie,
                                    owner: Rc::downgrade(b),
                                    owner_pid: pid,
                                    watchers: Vec::new(),
                                }))
                            })
                            .clone()
                    };
                    if !same {
                        let h = target.borrow_mut().handle_for(&node, ctx.as_ref());
                        let t = if ty == TYPE_BINDER { TYPE_HANDLE } else { TYPE_WEAK_HANDLE };
                        data[off..off + 4].copy_from_slice(&t.to_le_bytes());
                        data[off + 8..off + 16].copy_from_slice(&(h as u64).to_le_bytes());
                        data[off + 16..off + 24].copy_from_slice(&0u64.to_le_bytes());
                    }
                }
                TYPE_HANDLE | TYPE_WEAK_HANDLE => {
                    let h = u32_at(&data, off + 8);
                    let node = if h == 0 { ctx.clone() } else { b.borrow().refs.get(&h).cloned() };
                    let Some(node) = node else {
                        fail(b, Work::FailedReply);
                        return Ok(());
                    };
                    let owned_by_target = node.borrow().owner.upgrade().is_some_and(|o| Rc::ptr_eq(&o, &target));
                    if owned_by_target {
                        let n = node.borrow();
                        let t = if ty == TYPE_HANDLE { TYPE_BINDER } else { TYPE_WEAK_BINDER };
                        data[off..off + 4].copy_from_slice(&t.to_le_bytes());
                        data[off + 8..off + 16].copy_from_slice(&n.ptr.to_le_bytes());
                        data[off + 16..off + 24].copy_from_slice(&n.cookie.to_le_bytes());
                    } else if !same {
                        let nh = target.borrow_mut().handle_for(&node, ctx.as_ref());
                        data[off + 8..off + 16].copy_from_slice(&(nh as u64).to_le_bytes());
                    }
                }
                TYPE_FD => {
                    let fd = u32_at(&data, off + 8);
                    let file: FileRef = self.proc(c).file(fd as u64)?;
                    let proc = target.borrow().process.upgrade().ok_or(EINVAL)?;
                    let nfd = proc.borrow_mut().install_fd(file, false, 0)?;
                    data[off + 8..off + 12].copy_from_slice(&(nfd as u32).to_le_bytes());
                }
                _ => {
                    kprintln!("linux: binder: unsupported object type {:#x}", ty);
                    fail(b, Work::FailedReply);
                    return Ok(());
                }
            }
        }

        // Copy into the receiver's area.
        let total = data.len().div_ceil(8) * 8 + offsets.len();
        let mut tp = target.borrow_mut();
        let Some(at) = tp.alloc(total as u64) else {
            drop(tp);
            kprintln!("linux: binder: receiver has no room ({} bytes)", total);
            fail(b, Work::FailedReply);
            return Ok(());
        };
        let (obj, base, _) = tp.buf.clone().unwrap();
        let dlen = data.len().div_ceil(8) * 8;
        obj.borrow_mut().write(at, &data);
        obj.borrow_mut().write(at + dlen as u64, &offsets);
        let (target_ptr, cookie) = node.as_ref().map(|n| (n.borrow().ptr, n.borrow().cookie)).unwrap_or((0, 0));
        let sender_pid = self.proc(c).pid;
        let tr = TxnData {
            target: target_ptr,
            cookie,
            code,
            flags: flags & (TF_ONE_WAY | TF_STATUS_CODE),
            sender_pid,
            data_size: data.len() as u64,
            offsets_size: offsets.len() as u64,
            buffer: base + at,
            offsets: base + at + dlen as u64,
        };
        if reply {
            let (_, caller_tid) = reply_to.unwrap();
            tp.thread_todo.entry(caller_tid).or_default().push_back(Work::Reply(tr));
        } else if flags & TF_ONE_WAY != 0 {
            tp.todo.push_back(Work::Transaction { tr, reply_to: None });
        } else {
            let rt = (Rc::downgrade(b), tid);
            tp.waiting.push(rt.clone());
            tp.todo.push_back(Work::Transaction { tr, reply_to: Some(rt) });
        }
        drop(tp);
        b.borrow_mut().thread_todo.entry(tid).or_default().push_back(Work::Complete);
        let _ = node.map(|n| n.borrow().owner_pid);
        Ok(())
    }

    pub(crate) fn binder_file(&self, f: &FileRef) -> Option<ProcRef> {
        match &f.borrow().kind {
            FileKind::Binder(b) => Some(b.clone()),
            _ => None,
        }
    }
}
