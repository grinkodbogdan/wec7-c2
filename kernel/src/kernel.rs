//! Components, capabilities, IPC, interrupt delivery, scheduling and the
//! system-call interface.
//!
//! Every component (M1: one thread per component) has its own address
//! space and capability space. All authority is a capability in a slot;
//! every system call names the slot it acts through and is checked against
//! the capability's object type and rights.

use crate::arch::mmu::phys_to_virt;
use crate::arch::{self, gic, mmu, timer, ExtState, TrapFrame};
use crate::boot::{self, GrantKind, Restart};
use crate::global::Global;
use crate::mm::vspace::{AddressSpace, Prot};
use crate::mm::{frame, page_ceil, page_floor, PAGE_SIZE};
use crate::smp::{self, MAX_CPUS};
use crate::{elf, klog, linux, platform};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use flare_abi::layout::{
    USER_BOOTINFO, USER_IMAGE_BASE, USER_IMAGE_END, USER_MMIO_BASE, USER_MMIO_END, USER_STACK_SIZE, USER_STACK_TOP,
};
use flare_abi::{
    fixed_from_str, syscall as nr, BootInfo, CapDesc, CapKind, Error, Message, Rights, ABI_VERSION, BOOTINFO_MAGIC,
    LABEL_NOTIFY, MAX_CAPS,
};
use flare_cap::{CSpace, CapId, Cdt};

/// Kernel objects a capability can refer to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Object {
    Endpoint(usize),
    Mmio {
        device: usize,
        reg: usize,
    },
    Irq(u32),
    Klog,
    /// A DMA region: (component, slot) it was granted to.
    Dma(usize, usize),
    /// Shared memory `boot::SHM_REGIONS[n]`.
    Shm(usize),
    /// Linux input device `boot::INPUT_DEVICES[n]`.
    Input(usize),
    /// Linux serial device `boot::SERIAL_DEVICES[n]`.
    Serial(usize),
}

impl Object {
    fn kind(self) -> CapKind {
        match self {
            Object::Endpoint(_) => CapKind::Endpoint,
            Object::Mmio { .. } => CapKind::Mmio,
            Object::Irq(_) => CapKind::Irq,
            Object::Klog => CapKind::Klog,
            Object::Dma(..) | Object::Shm(_) => CapKind::Dma,
            Object::Input(_) => CapKind::Input,
            Object::Serial(_) => CapKind::Serial,
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
pub(crate) enum State {
    Ready,
    Running,
    Sending {
        ep: usize,
        call: bool,
    },
    Receiving {
        ep: usize,
    },
    AwaitingReply {
        server: usize,
    },
    /// A Linux process waiting inside a system call (re-run on events).
    LinuxWait,
    Dead,
}

pub(crate) struct Component {
    pub(crate) name: String,
    pub(crate) state: State,
    pub(crate) frame: TrapFrame,
    pub(crate) ext: Box<ExtState>,
    outgoing: Message,
    /// The caller whose CALL we received and owe a reply.
    reply_to: Option<usize>,
    cspace: CSpace<MAX_CAPS>,
    /// Shared by the threads of a Linux process.
    pub(crate) vspace: Option<Rc<RefCell<AddressSpace>>>,
    /// Index into `boot::COMPONENTS` for boot components.
    boot: Option<usize>,
    /// Linux personality state, for Linux processes.
    pub(crate) linux: Option<Box<linux::Task>>,
    /// Slot available for reuse (dynamic components only).
    pub(crate) free: bool,
    incarnation: u32,
    started: bool,
    restarts: u32,
    finished: bool,
    /// Stopped by the supervisor (not a failure).
    stopped: bool,
    /// Start again once its old address space is released (restart of a
    /// component that was running on another CPU).
    respawn: bool,
}

impl Component {
    fn new(name: &str, boot: Option<usize>) -> Component {
        Component {
            name: String::from(name),
            state: State::Dead,
            frame: TrapFrame::zeroed(),
            ext: Box::default(),
            outgoing: Message::default(),
            reply_to: None,
            cspace: CSpace::new(),
            vspace: None,
            boot,
            linux: None,
            free: false,
            incarnation: 0,
            started: false,
            restarts: 0,
            finished: false,
            stopped: false,
            respawn: false,
        }
    }
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
    pub(crate) comps: Vec<Component>,
    eps: Vec<Endpoint>,
    cdt: Cdt<KCap>,
    roots: BTreeMap<Object, CapId>,
    irqs: BTreeMap<u32, IrqBinding>,
    /// One-shot notifications (NOTIFY_AFTER): deadline (ns), endpoint, bit.
    alarms: Vec<(u64, usize, u32)>,
    klog_reader: Option<KlogReader>,
    ready: VecDeque<usize>,
    /// Per CPU: the component running there.
    current: [Option<usize>; MAX_CPUS],
    pub(crate) need_resched: [bool; MAX_CPUS],
    pub(crate) free_asids: Vec<u16>,
    pub(crate) halting: bool,
    ticks: u64,
    /// The Linux personality, if the composition starts one.
    pub(crate) linux: Option<linux::Personality>,
    /// Per CPU: whose FP/SIMD/TLS state is live in its registers.
    pub(crate) ext_owner: [Option<usize>; MAX_CPUS],
    /// Shared-memory regions (physical base, length), as `boot::SHM_REGIONS`.
    pub(crate) shm: Vec<(u64, u64)>,
    /// The display generation the kernel console follows (0: none).
    console_gen: u32,
    /// Byte streams between drivers and Linux devices, as `boot::SERIAL_DEVICES`.
    pub(crate) serials: Vec<SerialPort>,
}

/// One serial device: bytes from the driver to Linux (`rx`) and back (`tx`).
#[derive(Default)]
pub(crate) struct SerialPort {
    pub rx: VecDeque<u8>,
    pub tx: VecDeque<u8>,
    /// Endpoint and bit to raise when Linux writes.
    notify: Option<(usize, u32)>,
}

/// Bytes a serial direction may hold before writers see backpressure.
pub(crate) const SERIAL_BUF: usize = 1 << 20;

static KERNEL: Global<Option<Kernel>> = Global::new(None);
static BOOT_FRAMES: Global<[TrapFrame; MAX_CPUS]> = Global::new([TrapFrame::zeroed(); MAX_CPUS]);

fn kernel() -> &'static mut Kernel {
    // SAFETY: obtained once per kernel entry and threaded through by
    // reference; see `Global`.
    unsafe { KERNEL.get().as_mut().expect("kernel not started") }
}

type SysResult = Result<(), Error>;

/// Start the boot components and enter user space.
pub fn start() -> ! {
    let mut k = Kernel {
        comps: boot::COMPONENTS.iter().enumerate().map(|(i, bc)| Component::new(bc.name, Some(i))).collect(),
        eps: boot::ENDPOINTS.iter().map(|_| Endpoint::default()).collect(),
        cdt: Cdt::new(),
        roots: BTreeMap::new(),
        irqs: BTreeMap::new(),
        alarms: Vec::new(),
        klog_reader: None,
        ready: VecDeque::new(),
        current: [None; MAX_CPUS],
        need_resched: [false; MAX_CPUS],
        free_asids: (1..=255).rev().collect(),
        halting: false,
        ticks: 0,
        linux: None,
        ext_owner: [None; MAX_CPUS],
        shm: Vec::new(),
        console_gen: 0,
        serials: boot::SERIAL_DEVICES.iter().map(|_| SerialPort::default()).collect(),
    };
    for (name, size) in boot::SHM_REGIONS {
        let len = page_ceil(*size);
        match frame::alloc_contiguous(len / PAGE_SIZE) {
            Some(pa) => {
                kprintln!("boot: shared memory '{}': {} KiB at {:#x}", name, len / 1024, pa);
                k.shm.push((pa, len));
            }
            None => panic!("no contiguous memory for shared memory '{}'", name),
        }
    }
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
    k.linux_start();
    // SAFETY: the one and only initialization of the kernel state.
    unsafe { *KERNEL.get() = Some(k) };
    smp::start_secondaries();
    enter_user(0, arch::boot_stack_top())
}

/// A secondary CPU joins the scheduler (called with the kernel lock held).
pub fn secondary_start(id: usize) -> ! {
    enter_user(id, smp::stack_top(id))
}

fn enter_user(cpu: usize, stack_top: u64) -> ! {
    // SAFETY: each CPU uses only its own boot frame.
    let frame = unsafe { &mut BOOT_FRAMES.get()[cpu] };
    leave_loop(frame);
    smp::unlock();
    arch::enter_first(frame, stack_top)
}

/// Choose what this CPU runs next, idling (with the kernel lock released)
/// until there is something.
fn leave_loop(frame: &mut TrapFrame) {
    loop {
        if kernel().try_leave(frame) {
            return;
        }
        smp::idle_wait();
        kernel().handle_irqs();
    }
}

/// Synchronous exception from EL0: system call or fault.
pub fn on_user_sync(frame: &mut TrapFrame) {
    let k = kernel();
    let cur = k.current[smp::cpu()].expect("user exception without a current component");
    if k.comps[cur].state == State::Dead {
        leave_loop(frame); // killed by another CPU while running here
        return;
    }
    k.comps[cur].frame = *frame;
    let esr = arch::read_esr();
    if k.comps[cur].linux.is_some() {
        match esr >> 26 {
            0x15 => k.linux_syscall(cur),
            _ => k.linux_fault(cur, arch::read_far(), esr, frame.elr),
        }
        leave_loop(frame);
        return;
    }
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
    leave_loop(frame);
}

/// Interrupt taken while a component was running.
pub fn on_user_irq(frame: &mut TrapFrame) {
    let k = kernel();
    if let Some(cur) = k.current[smp::cpu()] {
        k.comps[cur].frame = *frame;
    }
    k.handle_irqs();
    leave_loop(frame);
}

impl Kernel {
    // ---------------------------------------------------------------
    // Scheduling
    // ---------------------------------------------------------------

    /// Pick the component to run next on this CPU and load its state into
    /// `frame`; `false` if there is nothing to run.
    fn try_leave(&mut self, frame: &mut TrapFrame) -> bool {
        loop {
            if !self.pick_next() {
                return false;
            }
            let c = self.current[smp::cpu()].expect("scheduler picked nothing");
            // Linux signals are delivered on the way back to user mode.
            if self.comps[c].linux.is_some() {
                self.linux_deliver(c);
                if self.comps[c].state != State::Running {
                    continue;
                }
            }
            break;
        }
        self.load(frame);
        true
    }

    /// Choose this CPU's next component; `false` if there is none (the
    /// caller idles and asks again).
    fn pick_next(&mut self) -> bool {
        let cpu = smp::cpu();
        self.linux_poll();
        self.flush_klog();
        if self.halting && self.console_drained() {
            self.halt();
        }
        if let Some(c) = self.current[cpu] {
            if self.comps[c].state == State::Dead {
                // Torn down by another CPU while it ran here.
                self.current[cpu] = None;
                self.ext_owner[cpu] = None;
                mmu::switch_to_kernel();
                self.release_vspace(c);
                if core::mem::take(&mut self.comps[c].respawn) {
                    self.respawn(c);
                }
            } else if self.comps[c].state == State::Running {
                if !(self.need_resched[cpu] && !self.ready.is_empty()) {
                    self.need_resched[cpu] = false;
                    return true;
                }
                self.comps[c].state = State::Ready;
                self.ready.push_back(c);
            }
        }
        self.need_resched[cpu] = false;
        while let Some(n) = self.ready.pop_front() {
            if self.comps[n].state == State::Ready {
                self.comps[n].state = State::Running;
                self.current[cpu] = Some(n);
                return true;
            }
        }
        // Nothing runnable here. Leave the registers of whoever ran last in
        // memory (it may resume on another CPU) and go idle.
        self.current[cpu] = None;
        if let Some(o) = self.ext_owner[cpu].take() {
            if self.comps[o].state != State::Dead {
                arch::ext_save(&mut self.comps[o].ext);
            }
        }
        false
    }

    fn load(&mut self, frame: &mut TrapFrame) {
        let cpu = smp::cpu();
        let c = self.current[cpu].expect("scheduler picked nothing");
        if self.ext_owner[cpu] != Some(c) {
            if let Some(o) = self.ext_owner[cpu] {
                if self.comps[o].state != State::Dead {
                    arch::ext_save(&mut self.comps[o].ext);
                }
            }
            arch::ext_restore(&self.comps[c].ext);
            self.ext_owner[cpu] = Some(c);
        }
        *frame = self.comps[c].frame;
        mmu::switch_to(self.comps[c].vspace.as_ref().expect("running component without address space").borrow().ttbr());
    }

    /// Save `c`'s FP/SIMD/TLS state if it is live in the CPU.
    pub(crate) fn save_ext(&mut self, c: usize) {
        if self.ext_owner[smp::cpu()] == Some(c) {
            arch::ext_save(&mut self.comps[c].ext);
        }
    }

    /// A slot for a new dynamic component (Linux process).
    pub(crate) fn alloc_comp(&mut self, name: &str) -> usize {
        let c = Component::new(name, None);
        let in_use = |i: usize| self.current.contains(&Some(i));
        match (0..self.comps.len()).find(|&i| self.comps[i].free && self.comps[i].vspace.is_none() && !in_use(i)) {
            Some(i) => {
                self.comps[i] = c;
                i
            }
            None => {
                self.comps.push(c);
                self.comps.len() - 1
            }
        }
    }

    pub(crate) fn make_ready(&mut self, c: usize) {
        self.comps[c].state = State::Ready;
        self.ready.push_back(c);
        smp::kick_idle();
    }

    /// Make `c` ready and let it run next (IPC hand-off).
    fn make_ready_next(&mut self, c: usize) {
        self.comps[c].state = State::Ready;
        self.ready.retain(|&x| x != c);
        self.ready.push_front(c);
        smp::kick_idle();
    }

    fn handle_irqs(&mut self) {
        while let Some(iar) = gic::ack() {
            let id = iar & 0x3ff;
            if id < 16 {
                // An inter-processor wake-up: nothing to do but look again.
            } else if id == timer::irq() {
                timer::rearm();
                if smp::cpu() == 0 {
                    self.ticks += 1;
                    self.fire_alarms();
                    self.console_follow_display();
                }
                self.need_resched[smp::cpu()] = true;
            } else if self.linux.is_some() && crate::linux::flarehw::granted_irq(id) {
                self.flarehw_irq_fired(id);
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

    /// When the display driver brings the panel online on its own memory (a
    /// phone: the boot loader's splash buffer is no longer scanned out), the
    /// kernel console moves there. Checked every tick; again if the driver
    /// restarts (a new generation).
    fn console_follow_display(&mut self) {
        let Some(i) = crate::boot::LINUX_DISPLAY else { return };
        if !crate::console::is_screen() {
            return;
        }
        let Some(&(pa, len)) = self.shm.get(i) else { return };
        // SAFETY: the display's shared memory, through the direct map; the
        // driver owns the header.
        let h = unsafe { core::ptr::read_volatile(crate::arch::mmu::phys_to_virt(pa) as *const flare_abi::proto::display::Header) };
        use flare_abi::proto::display as dp;
        if h.magic != dp::MAGIC || h.state != dp::STATE_ONLINE || h.format != dp::XRGB8888 || h.generation == self.console_gen {
            return;
        }
        if h.fb_offset as u64 + h.stride as u64 * h.height as u64 > len || h.stride < h.width * 4 {
            return;
        }
        self.console_gen = h.generation;
        crate::console::follow(
            (crate::arch::mmu::phys_to_virt(pa) + h.fb_offset as u64) as usize,
            h.width as usize,
            h.height as usize,
            h.stride as usize,
        );
        kprintln!("console: on the display driver's panel ({}x{}, generation {})", h.width, h.height, h.generation);
    }

    fn fire_alarms(&mut self) {
        if self.alarms.is_empty() {
            return;
        }
        let now = timer::now_ns();
        let mut due = Vec::new();
        self.alarms.retain(|&(t, ep, bit)| {
            if t <= now {
                due.push((ep, bit));
                false
            } else {
                true
            }
        });
        for (ep, bit) in due {
            self.notify(ep, bit);
        }
    }

    fn console_drained(&self) -> bool {
        match &self.klog_reader {
            None => true,
            Some(r) => klog::is_empty() && matches!(self.comps[r.comp].state, State::Receiving { .. }),
        }
    }

    pub(crate) fn check_halt(&mut self) {
        let linux_done = !boot::HALT_ON_LINUX_EXIT || self.linux.as_ref().is_none_or(|l| l.init_finished);
        let watched = !boot::HALT_WHEN_FINISHED.is_empty() || boot::HALT_ON_LINUX_EXIT;
        if !self.halting && watched && linux_done && boot::HALT_WHEN_FINISHED.iter().all(|&i| self.comps[i].finished) {
            self.halting = true;
            kprintln!("kernel: all watched components finished; halting once the console drains");
        }
    }

    fn halt(&mut self) -> ! {
        if self.klog_reader.take().is_some() {
            klog::set_reader_bound(false);
        }
        kprintln!(
            "flare: system halted after {} ticks on {} CPUs ({} capabilities live, {} frames in use)",
            self.ticks,
            smp::online(),
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

    fn load_memory(bc: &boot::BootComponent, vs: &mut AddressSpace) -> Result<(u64, u64), &'static str> {
        let entry = elf::load(bc.image, vs, USER_IMAGE_BASE..USER_IMAGE_END, USER_IMAGE_BASE)?.entry;
        let mut va = USER_STACK_TOP - USER_STACK_SIZE;
        while va < USER_STACK_TOP {
            vs.map_new(va, Prot::RW).map_err(|_| "cannot map stack")?;
            va += PAGE_SIZE;
        }
        let bi = vs.map_new(USER_BOOTINFO, Prot::R).map_err(|_| "cannot map boot info")?;
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
        let (entry, bi_pa) = match Self::load_memory(bc, &mut vs) {
            Ok(v) => v,
            Err(e) => {
                vs.destroy();
                self.free_asids.push(asid);
                return Err(e);
            }
        };
        {
            let c = &mut self.comps[i];
            if c.started {
                c.incarnation += 1;
            }
            c.started = true;
            c.vspace = Some(Rc::new(RefCell::new(vs)));
            *c.ext = Default::default();
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
        let bi = unsafe { &mut *(phys_to_virt(bi_pa) as *mut BootInfo) };
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
                GrantKind::Dma { .. } => (Object::Dma(i, slot), Rights::READ | Rights::WRITE | Rights::MAP, 0),
                GrantKind::Shm { region } => (Object::Shm(region), Rights::READ | Rights::WRITE | Rights::MAP, 0),
                GrantKind::Input { device } => (Object::Input(device), Rights::WRITE, 0),
                GrantKind::Serial { device } => (Object::Serial(device), Rights::READ | Rights::WRITE, 0),
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
                let mut vs = self.comps[i].vspace.as_ref().unwrap().borrow_mut();
                for off in (0..pend - pstart).step_by(PAGE_SIZE as usize) {
                    if vs.map_page(mmio_va + off, pstart + off, Prot::RW, true, false).is_err() {
                        drop(vs);
                        self.teardown(i);
                        return Err("cannot map device registers");
                    }
                }
                cap.mapping = Some((mmio_va, pend - pstart));
                desc.vaddr = mmio_va + (r.base - pstart);
                desc.size = r.size;
                mmio_va += pend - pstart;
            }
            if let GrantKind::Dma { size } = g.kind {
                // Zeroed pages owned by the mapping: freed when the
                // capability goes (unmapping an owned page frees it).
                let len = page_ceil(size);
                if mmio_va + len > USER_MMIO_END {
                    self.teardown(i);
                    return Err("MMIO window exhausted");
                }
                let mut vs = self.comps[i].vspace.as_ref().unwrap().borrow_mut();
                for off in (0..len).step_by(PAGE_SIZE as usize) {
                    if vs.map_new(mmio_va + off, Prot::RW).is_err() {
                        drop(vs);
                        self.teardown(i);
                        return Err("out of memory for DMA");
                    }
                }
                cap.mapping = Some((mmio_va, len));
                desc.vaddr = mmio_va;
                desc.size = size;
                mmio_va += len;
            }
            if let GrantKind::Shm { region } = g.kind {
                // The region's own pages, mapped unowned: they outlive
                // every holder.
                let (pa, len) = self.shm[region];
                if mmio_va + len > USER_MMIO_END {
                    self.teardown(i);
                    return Err("MMIO window exhausted");
                }
                let mut vs = self.comps[i].vspace.as_ref().unwrap().borrow_mut();
                for off in (0..len).step_by(PAGE_SIZE as usize) {
                    if vs.map_page(mmio_va + off, pa + off, Prot::RW, false, false).is_err() {
                        drop(vs);
                        self.teardown(i);
                        return Err("cannot map shared memory");
                    }
                }
                cap.mapping = Some((mmio_va, len));
                desc.vaddr = mmio_va;
                desc.size = len;
                mmio_va += len;
            }
            let root = self.root(obj);
            let id = self.cdt.derive(root, cap).expect("kernel root vanished");
            self.comps[i].cspace.set(slot, Some(id));
            bi.caps[slot] = desc;
        }
        self.make_ready(i);
        kprintln!(
            "kernel: started '{}' (incarnation {}, asid {}, {} capabilities)",
            bc.name,
            self.comps[i].incarnation,
            asid,
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
    pub(crate) fn teardown(&mut self, c: usize) {
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
        // Still running on another CPU: that CPU releases the address space
        // when it next enters the kernel (it may be executing in it now).
        let me = smp::cpu();
        if let Some(other) = (0..MAX_CPUS).find(|&i| i != me && self.current[i] == Some(c)) {
            for owner in self.ext_owner.iter_mut() {
                if *owner == Some(c) {
                    *owner = None;
                }
            }
            self.comps[c].state = State::Dead;
            gic::send_sgi(1 << other, smp::SGI_WAKE);
            return;
        }
        self.release_vspace(c);
        let comp = &mut self.comps[c];
        comp.state = State::Dead;
    }

    fn respawn(&mut self, c: usize) {
        let name = self.comps[c].name.clone();
        if let Err(e) = self.spawn(c) {
            kprintln!("supervisor: cannot start '{}': {}", name, e);
        }
    }

    /// Runtime management of boot components (the supervisor): drivers and
    /// services can be stopped, started and restarted while the system
    /// runs. A restart revokes everything the component holds and grants it
    /// afresh; its clients' calls in flight fail with E_PEER_DIED and are
    /// retried against the new incarnation (the endpoints survive).
    pub(crate) fn supervisor(&mut self, cmd: &str) -> String {
        let words: Vec<&str> = cmd.split_whitespace().collect();
        let find = |k: &Kernel, n: &str| (0..boot::COMPONENTS.len()).find(|&i| k.comps[i].name == n);
        match words.as_slice() {
            ["list"] | [] => {
                let mut out = String::from("instance     state      incarnation  restarts\n");
                for i in 0..boot::COMPONENTS.len() {
                    let c = &self.comps[i];
                    let state = match c.state {
                        State::Dead if c.stopped => "stopped",
                        State::Dead if c.finished => "finished",
                        State::Dead => "dead",
                        State::Running => "running",
                        _ => "waiting",
                    };
                    out.push_str(&format!("{:<12} {:<10} {:>11}  {:>8}\n", c.name, state, c.incarnation, c.restarts));
                }
                out
            }
            ["trace", "off"] => {
                if let Some(l) = self.linux.as_mut() {
                    l.trace = None;
                }
                "tracing off\n".into()
            }
            ["trace", prog] => match self.linux.as_mut() {
                Some(l) => {
                    l.trace = Some(String::from(*prog));
                    format!("tracing system calls of '{}' to the console (trace off to stop)\n", prog)
                }
                None => "no Linux personality\n".into(),
            },
            ["stop", n] => match find(self, n) {
                None => format!("no component '{}'\n", n),
                Some(i) if self.comps[i].state == State::Dead => format!("'{}' is not running\n", n),
                Some(i) => {
                    kprintln!("supervisor: stopping '{}'", n);
                    self.teardown(i);
                    self.comps[i].stopped = true;
                    format!("stopped '{}'\n", n)
                }
            },
            ["start", n] => match find(self, n) {
                None => format!("no component '{}'\n", n),
                Some(i) if self.comps[i].state != State::Dead => format!("'{}' is already running\n", n),
                Some(i) => {
                    kprintln!("supervisor: starting '{}'", n);
                    self.comps[i].stopped = false;
                    self.comps[i].finished = false;
                    match self.spawn(i) {
                        Ok(()) => format!("started '{}'\n", n),
                        Err(e) => format!("cannot start '{}': {}\n", n, e),
                    }
                }
            },
            ["restart", n] => match find(self, n) {
                None => format!("no component '{}'\n", n),
                Some(i) => {
                    kprintln!("supervisor: restarting '{}' (incarnation {})", n, self.comps[i].incarnation + 1);
                    if self.comps[i].state != State::Dead {
                        self.teardown(i);
                    }
                    self.comps[i].stopped = false;
                    self.comps[i].finished = false;
                    if self.comps[i].vspace.is_some() {
                        // Still running on another CPU: started once it lets go.
                        self.comps[i].respawn = true;
                    } else {
                        self.respawn(i);
                    }
                    format!("restarted '{}'\n", n)
                }
            },
            _ => String::from(
                "usage: list | start <instance> | stop <instance> | restart <instance> | trace <program>|off\n",
            ),
        }
    }

    /// Drop a component's address space (destroying it with its last user).
    fn release_vspace(&mut self, c: usize) {
        let comp = &mut self.comps[c];
        if let Some(vs) = comp.vspace.take() {
            // The last thread using an address space destroys it.
            if let Ok(vs) = Rc::try_unwrap(vs) {
                let vs = vs.into_inner();
                self.free_asids.push(vs.asid());
                vs.destroy();
            }
        }
        for owner in self.ext_owner.iter_mut() {
            if *owner == Some(c) {
                *owner = None;
            }
        }
    }

    /// Undo the side effects of a capability that has just been removed.
    fn undo(&mut self, id: CapId, v: KCap) {
        if let Some((o, s)) = v.owner {
            let comp = &mut self.comps[o];
            comp.cspace.clear_if(s, id);
            if let (Some((va, size)), Some(vs)) = (v.mapping, comp.vspace.as_ref()) {
                for off in (0..size).step_by(PAGE_SIZE as usize) {
                    vs.borrow_mut().unmap(va + off);
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
        let Some(b) = self.comps[c].boot else { return };
        let bc = &boot::COMPONENTS[b];
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

    pub(crate) fn copy_from_user(&self, c: usize, mut va: u64, len: usize) -> Result<Vec<u8>, Error> {
        let vs = self.comps[c].vspace.as_ref().ok_or(Error::BadAddress)?.borrow();
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let pa = vs.translate_user(va, false).ok_or(Error::BadAddress)?;
            let n = ((PAGE_SIZE - va % PAGE_SIZE) as usize).min(len - out.len());
            // SAFETY: `pa..pa+n` lies in one user-readable RAM page.
            out.extend_from_slice(unsafe { core::slice::from_raw_parts(phys_to_virt(pa) as *const u8, n) });
            va = va.checked_add(n as u64).ok_or(Error::BadAddress)?;
        }
        Ok(out)
    }

    pub(crate) fn copy_to_user(&self, c: usize, mut va: u64, data: &[u8]) -> SysResult {
        let vs = self.comps[c].vspace.as_ref().ok_or(Error::BadAddress)?.borrow();
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
            unsafe { core::ptr::copy_nonoverlapping(data[done..].as_ptr(), phys_to_virt(pa) as *mut u8, n) };
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
                self.need_resched[smp::cpu()] = true;
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
            nr::TTY_INPUT => self.sys_tty_input(c, a[0], a[1], a[2]),
            nr::DMA_PADDR => self.sys_dma_paddr(c, a[0], a[1]),
            nr::INPUT_REPORT => self.sys_input_report(c, a[0], a[1], a[2], a[3]),
            nr::SERIAL_IO => self.sys_serial_io(c, a[0], a[1], a[2], a[3]),
            nr::NOTIFY_AFTER => self.sys_notify_after(c, a[0], a[1], a[2]),
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

    fn sys_dma_paddr(&mut self, c: usize, slot: u64, offset: u64) -> SysResult {
        let (_, cap) = self.typed(c, slot, CapKind::Dma, Rights::NONE)?;
        let (va, size) = cap.mapping.ok_or(Error::InvalidArg)?;
        if offset >= size {
            return Err(Error::InvalidArg);
        }
        let pa = self.comps[c].vspace.as_ref().ok_or(Error::BadAddress)?.borrow().lookup(va + offset);
        let pa = pa.ok_or(Error::BadAddress)?;
        self.ret(c, &[pa]);
        Ok(())
    }

    fn sys_input_report(&mut self, c: usize, slot: u64, op: u64, ptr: u64, len: u64) -> SysResult {
        let (_, cap) = self.typed(c, slot, CapKind::Input, Rights::WRITE)?;
        let Object::Input(dev) = cap.obj else { return Err(Error::WrongType) };
        let bytes = self.copy_from_user(c, ptr, len.min(4096) as usize)?;
        if !self.linux_input_report(dev, op, &bytes) {
            return Err(Error::InvalidArg);
        }
        self.ret(c, &[]);
        Ok(())
    }

    fn sys_serial_io(&mut self, c: usize, slot: u64, op: u64, a: u64, b: u64) -> SysResult {
        let (_, cap) = self.typed(c, slot, CapKind::Serial, Rights::NONE)?;
        let Object::Serial(dev) = cap.obj else { return Err(Error::WrongType) };
        match op {
            0 => {
                let room = SERIAL_BUF.saturating_sub(self.serials[dev].rx.len());
                let bytes = self.copy_from_user(c, a, (b as usize).min(room).min(4096))?;
                self.serials[dev].rx.extend(bytes.iter());
                self.ret(c, &[bytes.len() as u64]);
            }
            1 => {
                let port = &mut self.serials[dev];
                let n = (b as usize).min(port.tx.len()).min(4096);
                let bytes: Vec<u8> = port.tx.drain(..n).collect();
                self.copy_to_user(c, a, &bytes)?;
                self.ret(c, &[n as u64]);
            }
            2 => {
                let (ep, _) = self.endpoint(c, a, Rights::RECV)?;
                if b >= 64 {
                    return Err(Error::InvalidArg);
                }
                self.serials[dev].notify = Some((ep, b as u32));
                if !self.serials[dev].tx.is_empty() {
                    self.notify(ep, b as u32);
                }
                self.ret(c, &[]);
            }
            _ => return Err(Error::InvalidArg),
        }
        Ok(())
    }

    /// Linux wrote to serial device `dev`: wake its driver.
    pub(crate) fn serial_kick(&mut self, dev: usize) {
        if let Some((ep, bit)) = self.serials[dev].notify {
            self.notify(ep, bit);
        }
    }

    fn sys_tty_input(&mut self, c: usize, slot: u64, ptr: u64, len: u64) -> SysResult {
        self.typed(c, slot, CapKind::Klog, Rights::WRITE)?;
        let bytes = self.copy_from_user(c, ptr, len.min(512) as usize)?;
        self.linux_tty_input(&bytes);
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

    fn sys_notify_after(&mut self, c: usize, ep_slot: u64, bit: u64, ns: u64) -> SysResult {
        let (ep, _) = self.endpoint(c, ep_slot, Rights::RECV)?;
        if bit >= 64 || self.alarms.len() >= 256 {
            return Err(Error::InvalidArg);
        }
        self.alarms.push((timer::now_ns().saturating_add(ns), ep, bit as u32));
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
