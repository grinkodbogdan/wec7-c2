//! Components, capabilities, IPC, interrupt delivery, scheduling and the
//! system-call interface.
//!
//! Every component (M1: one thread per component) has its own address
//! space and capability space. All authority is a capability in a slot;
//! every system call names the slot it acts through and is checked against
//! the capability's object type and rights.

use crate::arch::{self, gic, mmu, timer, TrapFrame};
use crate::boot::{self, GrantKind, Restart};
use crate::global::Global;
use crate::mm::vspace::{AddressSpace, MapKind};
use crate::mm::{frame, page_ceil, page_floor, PAGE_SIZE};
use crate::{elf, klog, platform};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use flare_abi::layout::{USER_BOOTINFO, USER_MMIO_BASE, USER_MMIO_END, USER_STACK_SIZE, USER_STACK_TOP};
use flare_abi::{
    fixed_from_str, syscall as nr, BootInfo, CapDesc, CapKind, Error, Message, Rights, ABI_VERSION, BOOTINFO_MAGIC,
    LABEL_NOTIFY, MAX_CAPS,
};
use flare_cap::{CSpace, CapId, Cdt};

/// Kernel objects a capability can refer to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Object {
    Endpoint(usize),
    Mmio { device: usize, reg: usize },
    Irq(u32),
    Klog,
}

impl Object {
    fn kind(self) -> CapKind {
        match self {
            Object::Endpoint(_) => CapKind::Endpoint,
            Object::Mmio { .. } => CapKind::Mmio,
            Object::Irq(_) => CapKind::Irq,
            Object::Klog => CapKind::Klog,
        }
    }
}

#[derive(Clone, Debug)]
struct KCap {
    obj: Object,
    rights: Rights,
    badge: u64,
    /// (component, slot) holding this capability; `None` for kernel roots.
    owner: Option<(usize, usize)>,
    /// User mapping created for this capability (MMIO): (va, size).
    mapping: Option<(u64, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ready,
    Running,
    Sending { ep: usize, call: bool },
    Receiving { ep: usize },
    AwaitingReply { server: usize },
    Dead,
}

struct Component {
    name: &'static str,
    state: State,
    frame: TrapFrame,
    outgoing: Message,
    /// The caller whose CALL we received and owe a reply.
    reply_to: Option<usize>,
    cspace: CSpace<MAX_CAPS>,
    vspace: Option<AddressSpace>,
    frames: Vec<u64>,
    incarnation: u32,
    started: bool,
    restarts: u32,
    finished: bool,
}

#[derive(Default)]
struct Endpoint {
    senders: VecDeque<usize>,
    receivers: VecDeque<usize>,
    /// Pending notification bits (interrupts, kernel log).
    pending: u64,
}

struct IrqBinding {
    ep: usize,
    bit: u32,
    cap: CapId,
    masked: bool,
}

struct KlogReader {
    ep: usize,
    bit: u32,
    cap: CapId,
    comp: usize,
}

pub struct Kernel {
    comps: Vec<Component>,
    eps: Vec<Endpoint>,
    cdt: Cdt<KCap>,
    roots: BTreeMap<Object, CapId>,
    irqs: BTreeMap<u32, IrqBinding>,
    klog_reader: Option<KlogReader>,
    ready: VecDeque<usize>,
    current: Option<usize>,
    need_resched: bool,
    free_asids: Vec<u16>,
    halting: bool,
    ticks: u64,
}

static KERNEL: Global<Option<Kernel>> = Global::new(None);
static BOOT_FRAME: Global<TrapFrame> = Global::new(TrapFrame::zeroed());

fn kernel() -> &'static mut Kernel {
    // SAFETY: obtained once per kernel entry and threaded through by
    // reference; see `Global`.
    unsafe { KERNEL.get().as_mut().expect("kernel not started") }
}

type SysResult = Result<(), Error>;

/// Start the boot components and enter user space.
pub fn start() -> ! {
    let mut k = Kernel {
        comps: boot::COMPONENTS
            .iter()
            .map(|bc| Component {
                name: bc.name,
                state: State::Dead,
                frame: TrapFrame::zeroed(),
                outgoing: Message::default(),
                reply_to: None,
                cspace: CSpace::new(),
                vspace: None,
                frames: Vec::new(),
                incarnation: 0,
                started: false,
                restarts: 0,
                finished: false,
            })
            .collect(),
        eps: boot::ENDPOINTS.iter().map(|_| Endpoint::default()).collect(),
        cdt: Cdt::new(),
        roots: BTreeMap::new(),
        irqs: BTreeMap::new(),
        klog_reader: None,
        ready: VecDeque::new(),
        current: None,
        need_resched: false,
        free_asids: (1..=255).rev().collect(),
        halting: false,
        ticks: 0,
    };
    kprintln!(
        "boot: system '{}': {} component(s), endpoints {:?}",
        boot::SYSTEM_NAME,
        boot::COMPONENTS.len(),
        boot::ENDPOINTS
    );
    for i in 0..k.comps.len() {
        if let Err(e) = k.spawn(i) {
            kprintln!("boot: cannot start '{}': {}", k.comps[i].name, e);
            k.comps[i].finished = true;
        }
    }
    // SAFETY: the one and only initialization of the kernel state.
    unsafe { *KERNEL.get() = Some(k) };
    // SAFETY: BOOT_FRAME is only used here.
    let frame = unsafe { BOOT_FRAME.get() };
    kernel().leave(frame);
    arch::enter_first(frame)
}

/// Synchronous exception from EL0: system call or fault.
pub fn on_user_sync(frame: &mut TrapFrame) {
    let k = kernel();
    let cur = k.current.expect("user exception without a current component");
    k.comps[cur].frame = *frame;
    let esr = arch::read_esr();
    match esr >> 26 {
        0x15 => k.syscall(cur),
        ec @ (0x20 | 0x24) => {
            let what = if ec == 0x24 { "data abort" } else { "instruction abort" };
            let kind = match (esr & 0x3f) >> 2 {
                1 => "translation fault",
                2 => "access flag fault",
                3 => "permission fault",
                _ => "fault",
            };
            k.fault(cur, format!("{} ({}) at {:#x}, pc {:#x}", what, kind, arch::read_far(), frame.elr));
        }
        ec => k.fault(cur, format!("unhandled exception class {:#x} at pc {:#x}", ec, frame.elr)),
    }
    k.leave(frame);
}

/// Interrupt taken while a component was running.
pub fn on_user_irq(frame: &mut TrapFrame) {
    let k = kernel();
    if let Some(cur) = k.current {
        k.comps[cur].frame = *frame;
    }
    k.handle_irqs();
    k.leave(frame);
}

impl Kernel {
    // ---------------------------------------------------------------
    // Scheduling
    // ---------------------------------------------------------------

    /// Pick the component to run next and load its state into `frame`.
    fn leave(&mut self, frame: &mut TrapFrame) {
        loop {
            self.flush_klog();
            if self.halting && self.console_drained() {
                self.halt();
            }
            if let Some(c) = self.current {
                if self.comps[c].state == State::Running {
                    if self.need_resched && !self.ready.is_empty() {
                        self.comps[c].state = State::Ready;
                        self.ready.push_back(c);
                    } else {
                        break;
                    }
                }
            }
            self.need_resched = false;
            let mut picked = None;
            while let Some(n) = self.ready.pop_front() {
                if self.comps[n].state == State::Ready {
                    picked = Some(n);
                    break;
                }
            }
            if let Some(n) = picked {
                self.comps[n].state = State::Running;
                self.current = Some(n);
                break;
            }
            // Nothing runnable: idle until an interrupt arrives.
            self.current = None;
            arch::wait_for_interrupt();
            self.handle_irqs();
        }
        self.need_resched = false;
        let c = self.current.expect("scheduler picked nothing");
        *frame = self.comps[c].frame;
        mmu::switch_to(self.comps[c].vspace.as_ref().expect("running component without address space").ttbr());
    }

    fn make_ready(&mut self, c: usize) {
        self.comps[c].state = State::Ready;
        self.ready.push_back(c);
    }

    /// Make `c` ready and let it run next (IPC hand-off).
    fn make_ready_next(&mut self, c: usize) {
        self.comps[c].state = State::Ready;
        self.ready.retain(|&x| x != c);
        self.ready.push_front(c);
    }

    fn handle_irqs(&mut self) {
        while let Some(iar) = gic::ack() {
            let id = iar & 0x3ff;
            if id == timer::irq() {
                timer::rearm();
                self.ticks += 1;
                self.need_resched = true;
            } else if let Some(b) = self.irqs.get_mut(&id) {
                // Level-triggered lines stay masked until the driver acks.
                gic::disable(id);
                b.masked = true;
                let (ep, bit) = (b.ep, b.bit);
                self.notify(ep, bit);
            } else {
                gic::disable(id);
                kprintln!("kernel: interrupt {} has no bound handler; masked", id);
            }
            gic::eoi(iar);
        }
    }

    fn console_drained(&self) -> bool {
        match &self.klog_reader {
            None => true,
            Some(r) => klog::is_empty() && matches!(self.comps[r.comp].state, State::Receiving { .. }),
        }
    }

    fn check_halt(&mut self) {
        if !self.halting
            && !boot::HALT_WHEN_FINISHED.is_empty()
            && boot::HALT_WHEN_FINISHED.iter().all(|&i| self.comps[i].finished)
        {
            self.halting = true;
            kprintln!("kernel: all watched components finished; halting once the console drains");
        }
    }

    fn halt(&mut self) -> ! {
        if self.klog_reader.take().is_some() {
            klog::set_reader_bound(false);
        }
        kprintln!(
            "flare: system halted after {} ticks ({} capabilities live, {} frames in use)",
            self.ticks,
            self.cdt.len(),
            frame::in_use()
        );
        arch::psci::system_off()
    }

    // ---------------------------------------------------------------
    // Component lifecycle
    // ---------------------------------------------------------------

    fn root(&mut self, obj: Object) -> CapId {
        if let Some(&id) = self.roots.get(&obj) {
            return id;
        }
        let id = self.cdt.insert_root(KCap { obj, rights: Rights::ALL, badge: 0, owner: None, mapping: None });
        self.roots.insert(obj, id);
        id
    }

    fn load_memory(
        bc: &boot::BootComponent,
        vs: &mut AddressSpace,
        frames: &mut Vec<u64>,
    ) -> Result<(u64, u64), &'static str> {
        let entry = elf::load(bc.image, vs, frames)?;
        let mut va = USER_STACK_TOP - USER_STACK_SIZE;
        while va < USER_STACK_TOP {
            let pa = frame::alloc().ok_or("out of memory")?;
            frames.push(pa);
            vs.map(va, pa, MapKind::Data).map_err(|_| "cannot map stack")?;
            va += PAGE_SIZE;
        }
        let bi = frame::alloc().ok_or("out of memory")?;
        frames.push(bi);
        vs.map(USER_BOOTINFO, bi, MapKind::ReadOnly).map_err(|_| "cannot map boot info")?;
        Ok((entry, bi))
    }

    /// Create (or re-create) component `i` from its boot description.
    fn spawn(&mut self, i: usize) -> Result<(), &'static str> {
        let bc = &boot::COMPONENTS[i];
        let asid = self.free_asids.pop().ok_or("out of address-space identifiers")?;
        let Some(mut vs) = AddressSpace::new(asid) else {
            self.free_asids.push(asid);
            return Err("out of memory");
        };
        let mut frames = Vec::new();
        let (entry, bi_pa) = match Self::load_memory(bc, &mut vs, &mut frames) {
            Ok(v) => v,
            Err(e) => {
                frames.into_iter().for_each(frame::free);
                vs.destroy();
                self.free_asids.push(asid);
                return Err(e);
            }
        };
        let page_count = frames.len();
        {
            let c = &mut self.comps[i];
            if c.started {
                c.incarnation += 1;
            }
            c.started = true;
            c.vspace = Some(vs);
            c.frames = frames;
            c.reply_to = None;
            c.cspace = CSpace::new();
            c.finished = false;
            c.frame = TrapFrame::zeroed();
            c.frame.x[0] = USER_BOOTINFO;
            c.frame.elr = entry;
            c.frame.sp_el0 = USER_STACK_TOP;
            c.frame.spsr = 0; // EL0t, interrupts unmasked
        }
        // SAFETY: freshly allocated, zeroed frame owned by this component.
        let bi = unsafe { &mut *(bi_pa as *mut BootInfo) };
        bi.magic = BOOTINFO_MAGIC;
        bi.abi_version = ABI_VERSION;
        bi.name = fixed_from_str(bc.name);
        bi.incarnation = self.comps[i].incarnation;
        bi.cap_count = bc.grants.len() as u32;

        let mut mmio_va = USER_MMIO_BASE;
        for (slot, g) in bc.grants.iter().enumerate() {
            let (obj, rights, badge) = match g.kind {
                GrantKind::Mmio { device, reg } => {
                    (Object::Mmio { device, reg }, Rights::READ | Rights::WRITE | Rights::MAP, 0)
                }
                GrantKind::Irq { intid } => (Object::Irq(intid), Rights::READ | Rights::WRITE, 0),
                GrantKind::Endpoint { endpoint, rights, badge } => (Object::Endpoint(endpoint), rights, badge),
                GrantKind::Klog { rights } => (Object::Klog, rights, 0),
            };
            let mut desc =
                CapDesc { slot: slot as u32, kind: obj.kind() as u32, vaddr: 0, size: 0, name: fixed_from_str(g.name) };
            let mut cap = KCap { obj, rights, badge, owner: Some((i, slot)), mapping: None };
            if let Object::Mmio { device, reg } = obj {
                let r = platform::DEVICES[device].regs[reg];
                let (pstart, pend) = (page_floor(r.base), page_ceil(r.base + r.size));
                if mmio_va + (pend - pstart) > USER_MMIO_END {
                    self.teardown(i);
                    return Err("MMIO window exhausted");
                }
                let vs = self.comps[i].vspace.as_mut().unwrap();
                for off in (0..pend - pstart).step_by(PAGE_SIZE as usize) {
                    if vs.map(mmio_va + off, pstart + off, MapKind::Device).is_err() {
                        self.teardown(i);
                        return Err("cannot map device registers");
                    }
                }
                cap.mapping = Some((mmio_va, pend - pstart));
                desc.vaddr = mmio_va + (r.base - pstart);
                desc.size = r.size;
                mmio_va += pend - pstart;
            }
            let root = self.root(obj);
            let id = self.cdt.derive(root, cap).expect("kernel root vanished");
            self.comps[i].cspace.set(slot, Some(id));
            bi.caps[slot] = desc;
        }
        self.make_ready(i);
        kprintln!(
            "kernel: started '{}' (incarnation {}, asid {}, {} pages, {} capabilities)",
            bc.name,
            self.comps[i].incarnation,
            asid,
            page_count,
            bc.grants.len()
        );
        Ok(())
    }

    /// Wake `c` with an error instead of the result it was waiting for.
    fn fail_waiter(&mut self, c: usize, e: Error) {
        self.comps[c].frame.x[0] = e as u64;
        self.make_ready(c);
    }

    /// Release everything component `c` holds.
    fn teardown(&mut self, c: usize) {
        for ep in &mut self.eps {
            ep.senders.retain(|&s| s != c);
            ep.receivers.retain(|&r| r != c);
        }
        self.ready.retain(|&r| r != c);
        self.comps[c].reply_to = None;
        for j in 0..self.comps.len() {
            if j == c {
                continue;
            }
            if self.comps[j].state == (State::AwaitingReply { server: c }) {
                self.fail_waiter(j, Error::PeerDied);
            }
            if self.comps[j].reply_to == Some(c) {
                self.comps[j].reply_to = None;
            }
        }
        // Authority: everything it holds, and everything derived from that.
        let held: Vec<CapId> = self.comps[c].cspace.occupied().map(|(_, id)| id).collect();
        for id in held {
            if let Ok(removed) = self.cdt.revoke(id) {
                for (rid, v) in removed {
                    self.undo(rid, v);
                }
            }
            if let Ok(v) = self.cdt.delete(id) {
                self.undo(id, v);
            }
        }
        let comp = &mut self.comps[c];
        if let Some(vs) = comp.vspace.take() {
            self.free_asids.push(vs.asid());
            vs.destroy();
        }
        for f in comp.frames.drain(..) {
            frame::free(f);
        }
        comp.state = State::Dead;
    }

    /// Undo the side effects of a capability that has just been removed.
    fn undo(&mut self, id: CapId, v: KCap) {
        if let Some((o, s)) = v.owner {
            let comp = &mut self.comps[o];
            comp.cspace.clear_if(s, id);
            if let (Some((va, size)), Some(vs)) = (v.mapping, comp.vspace.as_mut()) {
                for off in (0..size).step_by(PAGE_SIZE as usize) {
                    vs.unmap(va + off);
                }
            }
        }
        match v.obj {
            Object::Irq(n) if self.irqs.get(&n).map(|b| b.cap) == Some(id) => {
                self.irqs.remove(&n);
                gic::disable(n);
            }
            Object::Klog if self.klog_reader.as_ref().map(|r| r.cap) == Some(id) => {
                self.klog_reader = None;
                klog::set_reader_bound(false);
            }
            _ => {}
        }
    }

    fn fault(&mut self, c: usize, what: String) {
        kprintln!("kernel: component '{}' faulted: {}; stopping it", self.comps[c].name, what);
        self.finish(c, true);
    }

    fn exit(&mut self, c: usize, code: i64) {
        if code == 0 {
            kprintln!("kernel: component '{}' exited", self.comps[c].name);
        } else {
            kprintln!("kernel: component '{}' exited with code {}", self.comps[c].name, code);
        }
        self.finish(c, code != 0);
    }

    fn finish(&mut self, c: usize, failed: bool) {
        self.teardown(c);
        let bc = &boot::COMPONENTS[c];
        let name = bc.name;
        if failed && bc.restart == Restart::OnFailure && self.comps[c].restarts < bc.max_restarts {
            self.comps[c].restarts += 1;
            kprintln!("kernel: restarting '{}' ({}/{})", name, self.comps[c].restarts, bc.max_restarts);
            match self.spawn(c) {
                Ok(()) => return,
                Err(e) => kprintln!("kernel: restart of '{}' failed: {}", name, e),
            }
        }
        if failed {
            kprintln!("kernel: component '{}' retired", name);
        }
        self.comps[c].finished = true;
        self.check_halt();
    }

    // ---------------------------------------------------------------
    // IPC
    // ---------------------------------------------------------------

    /// Write a received message into `c`'s registers.
    fn write_msg(&mut self, c: usize, m: &Message) {
        let f = &mut self.comps[c].frame;
        f.x[0] = 0;
        f.x[1] = m.badge;
        f.x[2] = m.label;
        f.x[3..9].copy_from_slice(&m.words);
    }

    fn deliver(&mut self, c: usize, m: &Message) {
        self.write_msg(c, m);
        self.make_ready(c);
    }

    fn notify(&mut self, ep: usize, bit: u32) {
        self.eps[ep].pending |= 1 << bit;
        if let Some(r) = self.eps[ep].receivers.pop_front() {
            let bits = core::mem::take(&mut self.eps[ep].pending);
            self.deliver(r, &Message { badge: 0, label: LABEL_NOTIFY, words: [bits, 0, 0, 0, 0, 0] });
        }
    }

    fn flush_klog(&mut self) {
        if klog::take_dirty() {
            if let Some((ep, bit)) = self.klog_reader.as_ref().map(|r| (r.ep, r.bit)) {
                self.notify(ep, bit);
            }
        }
    }

    // ---------------------------------------------------------------
    // System calls
    // ---------------------------------------------------------------

    fn ret(&mut self, c: usize, vals: &[u64]) {
        let f = &mut self.comps[c].frame;
        f.x[0] = 0;
        f.x[1..1 + vals.len()].copy_from_slice(vals);
    }

    fn cap(&self, c: usize, slot: u64) -> Result<(CapId, KCap), Error> {
        let id =
            if slot < MAX_CAPS as u64 { self.comps[c].cspace.get(slot as usize) } else { None }.ok_or(Error::NoCap)?;
        let cap = self.cdt.get(id).ok_or(Error::NoCap)?;
        Ok((id, cap.clone()))
    }

    fn endpoint(&self, c: usize, slot: u64, need: Rights) -> Result<(usize, u64), Error> {
        let (_, cap) = self.cap(c, slot)?;
        let Object::Endpoint(ep) = cap.obj else { return Err(Error::WrongType) };
        if !cap.rights.contains(need) {
            return Err(Error::Rights);
        }
        Ok((ep, cap.badge))
    }

    fn typed(&self, c: usize, slot: u64, kind: CapKind, need: Rights) -> Result<(CapId, KCap), Error> {
        let (id, cap) = self.cap(c, slot)?;
        if cap.obj.kind() != kind {
            return Err(Error::WrongType);
        }
        if !cap.rights.contains(need) {
            return Err(Error::Rights);
        }
        Ok((id, cap))
    }

    fn copy_from_user(&self, c: usize, mut va: u64, len: usize) -> Result<Vec<u8>, Error> {
        let vs = self.comps[c].vspace.as_ref().ok_or(Error::BadAddress)?;
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let pa = vs.translate_user(va, false).ok_or(Error::BadAddress)?;
            let n = ((PAGE_SIZE - va % PAGE_SIZE) as usize).min(len - out.len());
            // SAFETY: `pa..pa+n` lies in one user-readable RAM page.
            out.extend_from_slice(unsafe { core::slice::from_raw_parts(pa as *const u8, n) });
            va = va.checked_add(n as u64).ok_or(Error::BadAddress)?;
        }
        Ok(out)
    }

    fn copy_to_user(&self, c: usize, mut va: u64, data: &[u8]) -> SysResult {
        let vs = self.comps[c].vspace.as_ref().ok_or(Error::BadAddress)?;
        // Validate the whole range first so a failure writes nothing.
        let mut check = va;
        let end = va.checked_add(data.len() as u64).ok_or(Error::BadAddress)?;
        while check < end {
            vs.translate_user(check, true).ok_or(Error::BadAddress)?;
            check = page_floor(check) + PAGE_SIZE;
        }
        let mut done = 0;
        while done < data.len() {
            let pa = vs.translate_user(va, true).ok_or(Error::BadAddress)?;
            let n = ((PAGE_SIZE - va % PAGE_SIZE) as usize).min(data.len() - done);
            // SAFETY: `pa..pa+n` lies in one user-writable RAM page.
            unsafe { core::ptr::copy_nonoverlapping(data[done..].as_ptr(), pa as *mut u8, n) };
            done += n;
            va += n as u64;
        }
        Ok(())
    }

    fn syscall(&mut self, c: usize) {
        let f = self.comps[c].frame;
        let a = [f.x[0], f.x[1], f.x[2], f.x[3], f.x[4], f.x[5], f.x[6], f.x[7]];
        let result = match f.x[8] {
            nr::DEBUG_WRITE => self.sys_debug_write(c, a[0], a[1], a[2]),
            nr::YIELD => {
                self.need_resched = true;
                self.ret(c, &[]);
                Ok(())
            }
            nr::SEND => self.sys_send(c, a, false),
            nr::CALL => self.sys_send(c, a, true),
            nr::RECV => self.sys_recv(c, a[0]),
            nr::REPLY => self.sys_reply(c, a),
            nr::CAP_DERIVE => self.sys_derive(c, a[0], a[1], a[2]),
            nr::CAP_REVOKE => self.sys_revoke(c, a[0]),
            nr::CAP_DELETE => self.sys_delete(c, a[0]),
            nr::IRQ_BIND => self.sys_irq_bind(c, a[0], a[1], a[2]),
            nr::IRQ_ACK => self.sys_irq_ack(c, a[0]),
            nr::KLOG_BIND => self.sys_klog_bind(c, a[0], a[1], a[2]),
            nr::KLOG_READ => self.sys_klog_read(c, a[0], a[1], a[2]),
            nr::EXIT => {
                self.exit(c, a[0] as i64);
                return;
            }
            _ => Err(Error::UnknownSyscall),
        };
        if let Err(e) = result {
            self.comps[c].frame.x[0] = e as u64;
        }
    }

    fn sys_debug_write(&mut self, c: usize, slot: u64, ptr: u64, len: u64) -> SysResult {
        self.typed(c, slot, CapKind::Klog, Rights::WRITE)?;
        let bytes = self.copy_from_user(c, ptr, len.min(512) as usize)?;
        klog::write(&bytes);
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_send(&mut self, c: usize, a: [u64; 8], call: bool) -> SysResult {
        let (ep, badge) = self.endpoint(c, a[0], Rights::SEND)?;
        if a[1] == LABEL_NOTIFY {
            return Err(Error::InvalidArg);
        }
        let msg = Message { badge, label: a[1], words: [a[2], a[3], a[4], a[5], a[6], a[7]] };
        if let Some(r) = self.eps[ep].receivers.pop_front() {
            self.write_msg(r, &msg);
            if call {
                self.comps[c].state = State::AwaitingReply { server: r };
                self.comps[r].reply_to = Some(c);
                self.make_ready_next(r);
            } else {
                self.make_ready(r);
                self.ret(c, &[]);
            }
        } else {
            self.comps[c].outgoing = msg;
            self.comps[c].state = State::Sending { ep, call };
            self.eps[ep].senders.push_back(c);
        }
        Ok(())
    }

    fn sys_recv(&mut self, c: usize, slot: u64) -> SysResult {
        let (ep, _) = self.endpoint(c, slot, Rights::RECV)?;
        // A server that receives again without replying drops the caller.
        if let Some(caller) = self.comps[c].reply_to.take() {
            if self.comps[caller].state == (State::AwaitingReply { server: c }) {
                self.fail_waiter(caller, Error::NoReply);
            }
        }
        if self.eps[ep].pending != 0 {
            let bits = core::mem::take(&mut self.eps[ep].pending);
            self.write_msg(c, &Message { badge: 0, label: LABEL_NOTIFY, words: [bits, 0, 0, 0, 0, 0] });
            return Ok(());
        }
        if let Some(s) = self.eps[ep].senders.pop_front() {
            let State::Sending { call, .. } = self.comps[s].state else { unreachable!("queued sender is not sending") };
            let msg = self.comps[s].outgoing;
            self.write_msg(c, &msg);
            if call {
                self.comps[s].state = State::AwaitingReply { server: c };
                self.comps[c].reply_to = Some(s);
            } else {
                self.comps[s].frame.x[0] = 0;
                self.make_ready(s);
            }
        } else {
            self.comps[c].state = State::Receiving { ep };
            self.eps[ep].receivers.push_back(c);
        }
        Ok(())
    }

    fn sys_reply(&mut self, c: usize, a: [u64; 8]) -> SysResult {
        let caller = self.comps[c].reply_to.take().ok_or(Error::NoReply)?;
        if self.comps[caller].state != (State::AwaitingReply { server: c }) {
            return Err(Error::NoReply);
        }
        self.deliver(caller, &Message { badge: 0, label: a[0], words: [a[1], a[2], a[3], a[4], a[5], a[6]] });
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_derive(&mut self, c: usize, slot: u64, rights: u64, badge: u64) -> SysResult {
        let (id, cap) = self.cap(c, slot)?;
        let rights = Rights(u32::try_from(rights).map_err(|_| Error::InvalidArg)?);
        if !rights.is_valid() {
            return Err(Error::InvalidArg);
        }
        if !rights.is_subset_of(cap.rights) {
            return Err(Error::Rights);
        }
        if badge != 0 && (cap.badge != 0 || !matches!(cap.obj, Object::Endpoint(_))) {
            return Err(Error::InvalidArg);
        }
        let ns = self.comps[c].cspace.first_free().ok_or(Error::NoSlot)?;
        let child = KCap {
            obj: cap.obj,
            rights,
            badge: if badge != 0 { badge } else { cap.badge },
            owner: Some((c, ns)),
            mapping: None,
        };
        let nid = self.cdt.derive(id, child).map_err(|_| Error::NoCap)?;
        self.comps[c].cspace.set(ns, Some(nid));
        self.ret(c, &[ns as u64]);
        Ok(())
    }

    fn sys_revoke(&mut self, c: usize, slot: u64) -> SysResult {
        let (id, _) = self.cap(c, slot)?;
        for (rid, v) in self.cdt.revoke(id).map_err(|_| Error::NoCap)? {
            self.undo(rid, v);
        }
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_delete(&mut self, c: usize, slot: u64) -> SysResult {
        let (id, _) = self.cap(c, slot)?;
        let v = self.cdt.delete(id).map_err(|_| Error::NoCap)?;
        self.undo(id, v);
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_irq_bind(&mut self, c: usize, irq_slot: u64, ep_slot: u64, bit: u64) -> SysResult {
        let (irq_id, cap) = self.typed(c, irq_slot, CapKind::Irq, Rights::NONE)?;
        let Object::Irq(n) = cap.obj else { return Err(Error::WrongType) };
        let (ep, _) = self.endpoint(c, ep_slot, Rights::RECV)?;
        if bit >= 64 {
            return Err(Error::InvalidArg);
        }
        if let Some(b) = self.irqs.get(&n) {
            if b.cap != irq_id && self.cdt.contains(b.cap) {
                return Err(Error::InvalidArg);
            }
        }
        self.irqs.insert(n, IrqBinding { ep, bit: bit as u32, cap: irq_id, masked: false });
        gic::enable(n);
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_irq_ack(&mut self, c: usize, slot: u64) -> SysResult {
        let (id, cap) = self.typed(c, slot, CapKind::Irq, Rights::NONE)?;
        let Object::Irq(n) = cap.obj else { return Err(Error::WrongType) };
        let b = self.irqs.get_mut(&n).filter(|b| b.cap == id).ok_or(Error::InvalidArg)?;
        if b.masked {
            b.masked = false;
            gic::enable(n);
        }
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_klog_bind(&mut self, c: usize, slot: u64, ep_slot: u64, bit: u64) -> SysResult {
        let (id, _) = self.typed(c, slot, CapKind::Klog, Rights::READ)?;
        let (ep, _) = self.endpoint(c, ep_slot, Rights::RECV)?;
        if bit >= 64 {
            return Err(Error::InvalidArg);
        }
        if let Some(r) = &self.klog_reader {
            if r.cap != id && self.cdt.contains(r.cap) {
                return Err(Error::InvalidArg);
            }
        }
        self.klog_reader = Some(KlogReader { ep, bit: bit as u32, cap: id, comp: c });
        klog::set_reader_bound(true);
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_klog_read(&mut self, c: usize, slot: u64, ptr: u64, len: u64) -> SysResult {
        self.typed(c, slot, CapKind::Klog, Rights::READ)?;
        let mut buf = [0u8; 512];
        let n = klog::peek(&mut buf[..(len as usize).min(512)]);
        self.copy_to_user(c, ptr, &buf[..n])?;
        klog::consume(n);
        self.ret(c, &[n as u64]);
        Ok(())
    }
}
