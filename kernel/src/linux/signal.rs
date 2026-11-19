//! POSIX signals for the Linux personality, delivered as Linux arm64 does.
//!
//! A pending, unblocked signal with a handler gets an `rt_sigframe` on the
//! user stack when its thread next returns to user mode: siginfo, then a
//! ucontext whose mcontext holds x0-x30, sp, pc, pstate and, in the
//! reserved area, an `fpsimd_context` (and an `esr_context` for faults).
//! The handler returns through x30 to the libc restorer or to the kernel's
//! sigreturn page. The layout follows QRT's `src/user/signal.c`
//! (github.com/grinkodbogdan/qrt), which mirrors Linux byte for byte.

use super::abi::*;
use super::{Kernel, Out, SIGTRAMP};
use crate::arch::timer;
use crate::kernel::State;
use alloc::vec::Vec;

pub const SA_RESTORER: u64 = 0x0400_0000;
pub const SA_ONSTACK: u64 = 0x0800_0000;
pub const SA_RESTART: u64 = 0x1000_0000;
pub const SA_NODEFER: u64 = 0x4000_0000;
pub const SA_RESETHAND: u64 = 0x8000_0000;
const SS_ONSTACK: i32 = 1;
const SS_DISABLE: i32 = 2;
const FPSIMD_MAGIC: u32 = 0x4650_8001;
const ESR_MAGIC: u32 = 0x4553_5201;

// rt_sigframe layout
const INFO: u64 = 0;
const UC: u64 = 128;
const MC: u64 = UC + 176;
const FRAME_SIZE: u64 = 128 + 176 + 288 + 4096;

pub const fn bit(sig: u64) -> u64 {
    1 << (sig - 1)
}
const UNBLOCKABLE: u64 = bit(SIGKILL) | bit(SIGSTOP);

pub const SI_USER: i32 = 0;
pub const SI_KERNEL: i32 = 0x80;
pub const SI_TKILL: i32 = -6;
pub const CLD_EXITED: i32 = 1;
pub const CLD_KILLED: i32 = 2;
pub const CLD_DUMPED: i32 = 3;

#[derive(Clone, Copy, Default)]
pub struct SigInfo {
    pub signo: i32,
    pub code: i32,
    pub pid: u32,
    pub uid: u32,
    pub addr: u64,
    pub status: i32,
    pub value: u64,
}

impl SigInfo {
    pub fn user(sig: u64, pid: u32) -> SigInfo {
        SigInfo { signo: sig as i32, code: SI_USER, pid, ..Default::default() }
    }

    fn to_bytes(self) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[0..4].copy_from_slice(&self.signo.to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        match self.signo as u64 {
            SIGSEGV | 4 | 7 | 5 | 8 if self.code > 0 => b[16..24].copy_from_slice(&self.addr.to_le_bytes()),
            SIGCHLD => {
                b[16..20].copy_from_slice(&self.pid.to_le_bytes());
                b[20..24].copy_from_slice(&self.uid.to_le_bytes());
                b[24..28].copy_from_slice(&self.status.to_le_bytes());
            }
            _ => {
                b[16..20].copy_from_slice(&self.pid.to_le_bytes());
                b[20..24].copy_from_slice(&self.uid.to_le_bytes());
                b[24..32].copy_from_slice(&self.value.to_le_bytes());
            }
        }
        b
    }

    /// struct signalfd_siginfo
    pub fn to_signalfd(self) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[0..4].copy_from_slice(&(self.signo as u32).to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        b[12..16].copy_from_slice(&self.pid.to_le_bytes());
        b[16..20].copy_from_slice(&self.uid.to_le_bytes());
        b[40..44].copy_from_slice(&self.status.to_le_bytes());
        b[48..56].copy_from_slice(&self.value.to_le_bytes());
        b[72..80].copy_from_slice(&self.addr.to_le_bytes());
        b
    }
}

#[derive(Clone, Copy, Default)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: u64,
}

/// Process-wide signal state (shared by all threads).
#[derive(Clone)]
pub struct ProcSig {
    pub actions: [SigAction; 65],
    pub pending: u64,
    pub infos: [SigInfo; 65],
    /// ITIMER_REAL: (deadline ns, interval ns).
    pub itimer: Option<(u64, u64)>,
}

impl Default for ProcSig {
    fn default() -> Self {
        ProcSig { actions: [SigAction::default(); 65], pending: 0, infos: [SigInfo::default(); 65], itimer: None }
    }
}

/// Per-thread signal state.
#[derive(Clone)]
pub struct ThreadSig {
    pub mask: u64,
    pub pending: u64,
    pub infos: [SigInfo; 65],
    /// Mask to restore after a handler run from sigsuspend/ppoll.
    pub saved_mask: Option<u64>,
    pub alt_sp: u64,
    pub alt_size: u64,
}

impl Default for ThreadSig {
    fn default() -> Self {
        ThreadSig { mask: 0, pending: 0, infos: [SigInfo::default(); 65], saved_mask: None, alt_sp: 0, alt_size: 0 }
    }
}

pub enum Default_ {
    Ignore,
    Terminate,
    Core,
}

pub fn default_action(sig: u64) -> Default_ {
    match sig {
        SIGCHLD | SIGCONT | SIGURG | SIGWINCH | SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => Default_::Ignore,
        3 | 4 | 5 | 6 | 7 | 8 | 11 | 24 | 25 | 31 => Default_::Core,
        _ => Default_::Terminate,
    }
}

/// System calls that are not restarted after a handler (Linux returns EINTR).
fn restartable(nr: u64) -> bool {
    !matches!(
        nr,
        nr::PPOLL
            | nr::PSELECT6
            | nr::EPOLL_PWAIT
            | nr::NANOSLEEP
            | nr::CLOCK_NANOSLEEP
            | nr::RT_SIGSUSPEND
            | nr::RT_SIGTIMEDWAIT
    )
}

impl Kernel {
    /// Is `sig` ignored by the process (so generating it does nothing)?
    fn sig_ignored(&self, c: usize, sig: u64) -> bool {
        let h = self.proc(c).sig.actions[sig as usize].handler;
        h == SIG_IGN || (h == SIG_DFL && matches!(default_action(sig), Default_::Ignore))
    }

    /// Generate a process-directed signal for the process of task `c`.
    pub(crate) fn sig_send_proc(&mut self, c: usize, info: SigInfo) {
        let sig = info.signo as u64;
        if sig == 0 || sig > 64 {
            return;
        }
        if sig == SIGKILL {
            let st = SIGKILL as i32;
            self.linux_exit_group(c, st);
            return;
        }
        if self.sig_ignored(c, sig) {
            return;
        }
        let mut p = self.proc(c);
        if p.sig.pending & bit(sig) == 0 {
            p.sig.infos[sig as usize] = info;
        }
        p.sig.pending |= bit(sig);
    }

    /// Generate a thread-directed signal.
    pub(crate) fn sig_send_thread(&mut self, c: usize, info: SigInfo) {
        let sig = info.signo as u64;
        if sig == 0 || sig > 64 {
            return;
        }
        if sig == SIGKILL {
            self.linux_exit_group(c, SIGKILL as i32);
            return;
        }
        if self.sig_ignored(c, sig) {
            return;
        }
        let t = self.task(c);
        if t.sig.pending & bit(sig) == 0 {
            t.sig.infos[sig as usize] = info;
        }
        t.sig.pending |= bit(sig);
    }

    /// Signals task `c` could take now.
    pub(crate) fn sig_deliverable(&self, c: usize) -> u64 {
        let t = self.comps[c].linux.as_ref().unwrap();
        (t.sig.pending | t.p.borrow().sig.pending) & !t.sig.mask
    }

    /// Take a pending signal (thread first, then process) among `set`.
    pub(crate) fn sig_take(&mut self, c: usize, set: u64) -> Option<SigInfo> {
        let t = self.comps[c].linux.as_mut().unwrap();
        let tp = t.sig.pending & set;
        if tp != 0 {
            let s = tp.trailing_zeros() as u64 + 1;
            t.sig.pending &= !bit(s);
            return Some(t.sig.infos[s as usize]);
        }
        let mut p = t.p.borrow_mut();
        let pp = p.sig.pending & set;
        if pp != 0 {
            let s = pp.trailing_zeros() as u64 + 1;
            p.sig.pending &= !bit(s);
            return Some(p.sig.infos[s as usize]);
        }
        None
    }

    /// A synchronous fault: deliver to a handler if there is one, else die.
    pub(crate) fn linux_force_signal(&mut self, c: usize, sig: u64, code: i32, addr: u64, esr: u64, pc: u64) {
        let action = self.proc(c).sig.actions[sig as usize];
        let blocked = self.task(c).sig.mask & bit(sig) != 0;
        if action.handler > SIG_IGN && !blocked {
            let info = SigInfo { signo: sig as i32, code, addr, ..Default::default() };
            if self.setup_frame(c, sig, info, esr, addr) {
                return;
            }
        }
        let (pid, name) = (self.proc(c).pid, self.comps[c].name.clone());
        kprintln!(
            "linux: pid {} ({}) killed by {}: esr {:#x} addr {:#x} pc {:#x}",
            pid,
            name,
            match sig {
                4 => "SIGILL",
                5 => "SIGTRAP",
                7 => "SIGBUS",
                _ => "SIGSEGV",
            },
            esr,
            addr,
            pc
        );
        self.linux_exit_group(c, sig as i32 | 0x80);
    }

    /// Called whenever task `c` is about to return to user mode.
    pub(crate) fn linux_deliver(&mut self, c: usize) {
        loop {
            let pend = self.sig_deliverable(c);
            if pend == 0 {
                return;
            }
            let s = pend.trailing_zeros() as u64 + 1;
            let Some(info) = self.sig_take(c, bit(s)) else { return };
            let action = self.proc(c).sig.actions[s as usize];
            match action.handler {
                SIG_IGN => continue,
                SIG_DFL => match default_action(s) {
                    Default_::Ignore => continue,
                    Default_::Terminate => {
                        self.linux_exit_group(c, s as i32);
                        return;
                    }
                    Default_::Core => {
                        self.linux_exit_group(c, s as i32 | 0x80);
                        return;
                    }
                },
                _ => {
                    if !self.setup_frame(c, s, info, 0, 0) {
                        self.linux_exit_group(c, SIGSEGV as i32 | 0x80);
                    }
                    return;
                }
            }
        }
    }

    fn setup_frame(&mut self, c: usize, sig: u64, info: SigInfo, esr: u64, far: u64) -> bool {
        self.save_ext(c);
        let action = self.proc(c).sig.actions[sig as usize];
        let f = self.comps[c].frame;
        let (alt_sp, alt_size, old_mask) = {
            let t = self.task(c);
            let old = t.sig.saved_mask.take().unwrap_or(t.sig.mask);
            (t.sig.alt_sp, t.sig.alt_size, old)
        };
        let on_alt = alt_size != 0 && f.sp_el0 > alt_sp && f.sp_el0 <= alt_sp + alt_size;
        let mut sp = f.sp_el0;
        if action.flags & SA_ONSTACK != 0 && alt_size != 0 && !on_alt {
            sp = alt_sp + alt_size;
        }
        let sp = (sp - FRAME_SIZE - 16) & !15;
        let mut fr: Vec<u8> = alloc::vec![0; (FRAME_SIZE + 16) as usize];
        let put = |fr: &mut Vec<u8>, off: u64, v: &[u8]| fr[off as usize..off as usize + v.len()].copy_from_slice(v);
        put(&mut fr, INFO, &info.to_bytes());
        // uc_stack
        put(&mut fr, UC + 16, &alt_sp.to_le_bytes());
        let ss_flags = if alt_size == 0 {
            SS_DISABLE
        } else if on_alt {
            SS_ONSTACK
        } else {
            0
        };
        put(&mut fr, UC + 24, &ss_flags.to_le_bytes());
        put(&mut fr, UC + 32, &alt_size.to_le_bytes());
        put(&mut fr, UC + 40, &old_mask.to_le_bytes());
        // mcontext
        put(&mut fr, MC, &far.to_le_bytes());
        for i in 0..31 {
            put(&mut fr, MC + 8 + i * 8, &f.x[i as usize].to_le_bytes());
        }
        put(&mut fr, MC + 256, &f.sp_el0.to_le_bytes());
        put(&mut fr, MC + 264, &f.elr.to_le_bytes());
        put(&mut fr, MC + 272, &f.spsr.to_le_bytes());
        let r = MC + 288;
        put(&mut fr, r, &FPSIMD_MAGIC.to_le_bytes());
        put(&mut fr, r + 4, &528u32.to_le_bytes());
        let ext = self.comps[c].ext.clone();
        put(&mut fr, r + 8, &(ext.fpsr as u32).to_le_bytes());
        put(&mut fr, r + 12, &(ext.fpcr as u32).to_le_bytes());
        for i in 0..32 {
            put(&mut fr, r + 16 + i * 16, &ext.v[i as usize].to_le_bytes());
        }
        if esr != 0 {
            put(&mut fr, r + 528, &ESR_MAGIC.to_le_bytes());
            put(&mut fr, r + 532, &16u32.to_le_bytes());
            put(&mut fr, r + 536, &esr.to_le_bytes());
        }
        // A frame record so backtraces continue through the handler.
        put(&mut fr, FRAME_SIZE, &f.x[29].to_le_bytes());
        put(&mut fr, FRAME_SIZE + 8, &f.x[30].to_le_bytes());
        if self.uwrite(c, sp, &fr).is_err() {
            return false;
        }
        let ret = if action.flags & SA_RESTORER != 0 && action.restorer != 0 { action.restorer } else { SIGTRAMP };
        let frame = &mut self.comps[c].frame;
        frame.sp_el0 = sp;
        frame.x[29] = sp + FRAME_SIZE;
        frame.x[30] = ret;
        frame.elr = action.handler;
        frame.x[0] = sig;
        frame.x[1] = sp + INFO;
        frame.x[2] = sp + UC;
        frame.spsr &= !(1 << 21);
        let t = self.task(c);
        let add = if action.flags & SA_NODEFER != 0 { 0 } else { bit(sig) };
        t.sig.mask |= (action.mask | add) & !UNBLOCKABLE;
        if action.flags & SA_RESETHAND != 0 {
            self.proc(c).sig.actions[sig as usize] = SigAction::default();
        }
        true
    }

    pub(crate) fn sys_rt_sigreturn(&mut self, c: usize) -> Out {
        let sp = self.comps[c].frame.sp_el0;
        let Ok(fr) = self.uread(c, sp, (MC + 288 + 528) as usize) else {
            self.linux_exit_group(c, SIGSEGV as i32 | 0x80);
            return Out::Gone;
        };
        let get = |off: u64| u64::from_le_bytes(fr[off as usize..off as usize + 8].try_into().unwrap());
        let mask = get(UC + 40);
        let frame = &mut self.comps[c].frame;
        for i in 0..31 {
            frame.x[i] = get(MC + 8 + i as u64 * 8);
        }
        frame.sp_el0 = get(MC + 256);
        frame.elr = get(MC + 264);
        frame.spsr = get(MC + 272) & 0xf000_0000;
        let r = (MC + 288) as usize;
        if u32::from_le_bytes(fr[r..r + 4].try_into().unwrap()) == FPSIMD_MAGIC {
            self.save_ext(c);
            let ext = &mut self.comps[c].ext;
            ext.fpsr = u32::from_le_bytes(fr[r + 8..r + 12].try_into().unwrap()) as u64;
            ext.fpcr = u32::from_le_bytes(fr[r + 12..r + 16].try_into().unwrap()) as u64;
            for i in 0..32 {
                let o = r + 16 + i * 16;
                ext.v[i] = u128::from_le_bytes(fr[o..o + 16].try_into().unwrap());
            }
            if self.ext_owner == Some(c) {
                self.ext_owner = None;
            }
        }
        self.task(c).sig.mask = mask & !UNBLOCKABLE;
        Out::Gone
    }

    /// A blocked task with a deliverable signal: abandon or restart its call.
    pub(crate) fn linux_interrupt(&mut self, c: usize) -> bool {
        let pend = self.sig_deliverable(c);
        if pend == 0 {
            return false;
        }
        let s = pend.trailing_zeros() as u64 + 1;
        let action = self.proc(c).sig.actions[s as usize];
        let nr = self.comps[c].frame.x[8];
        let handled = action.handler > SIG_IGN;
        if !handled && !matches!(default_action(s), Default_::Terminate | Default_::Core) {
            // Ignored by default: drop it and keep waiting.
            self.sig_take(c, bit(s));
            return false;
        }
        {
            let t = self.task(c);
            t.wait = Default::default();
            t.vfork_child = None;
        }
        if handled && action.flags & SA_RESTART != 0 && restartable(nr) {
            self.comps[c].frame.elr -= 4; // re-issue the svc after the handler
        } else {
            self.comps[c].frame.x[0] = (-EINTR) as u64;
        }
        self.make_ready(c);
        true
    }

    /// Expire interval timers (ITIMER_REAL).
    pub(crate) fn linux_timers(&mut self) {
        let now = timer::now_ns();
        let leaders: Vec<usize> = (0..self.comps.len())
            .filter(|&i| !self.comps[i].free && self.comps[i].state != State::Dead && self.comps[i].linux.is_some())
            .filter(|&i| {
                let t = self.comps[i].linux.as_ref().unwrap();
                t.tid == t.p.borrow().pid
            })
            .collect();
        for c in leaders {
            let fire = {
                let mut p = self.proc(c);
                match p.sig.itimer {
                    Some((d, iv)) if now >= d => {
                        p.sig.itimer = if iv > 0 { Some((d + iv * (1 + (now - d) / iv), iv)) } else { None };
                        true
                    }
                    _ => false,
                }
            };
            if fire {
                let info = SigInfo { signo: 14, code: SI_KERNEL, ..Default::default() };
                self.sig_send_proc(c, info);
            }
        }
    }

    // ----- system calls -----

    pub(crate) fn sys_sigaction(&mut self, c: usize, sig: u64, act: u64, oact: u64) -> Result<i64, i64> {
        if sig == 0 || sig > 64 || ((sig == SIGKILL || sig == SIGSTOP) && act != 0) {
            return Err(EINVAL);
        }
        let old = self.proc(c).sig.actions[sig as usize];
        if act != 0 {
            let b = self.uread(c, act, 32)?;
            let w = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
            let new = SigAction { handler: w(0), flags: w(1), restorer: w(2), mask: w(3) };
            let mut p = self.proc(c);
            p.sig.actions[sig as usize] = new;
            // Setting a signal to be ignored discards it.
            if new.handler == SIG_IGN || (new.handler == SIG_DFL && matches!(default_action(sig), Default_::Ignore)) {
                p.sig.pending &= !bit(sig);
            }
        }
        if oact != 0 {
            let mut b = [0u8; 32];
            b[0..8].copy_from_slice(&old.handler.to_le_bytes());
            b[8..16].copy_from_slice(&old.flags.to_le_bytes());
            b[16..24].copy_from_slice(&old.restorer.to_le_bytes());
            b[24..32].copy_from_slice(&old.mask.to_le_bytes());
            self.uwrite(c, oact, &b)?;
        }
        Ok(0)
    }

    pub(crate) fn sys_sigprocmask(&mut self, c: usize, how: u64, set: u64, oset: u64) -> Result<i64, i64> {
        let old = self.task(c).sig.mask;
        if set != 0 {
            let s = self.uread_u64(c, set)?;
            let new = match how {
                0 => old | s,
                1 => old & !s,
                2 => s,
                _ => return Err(EINVAL),
            };
            self.task(c).sig.mask = new & !UNBLOCKABLE;
        }
        if oset != 0 {
            self.uwrite(c, oset, &old.to_le_bytes())?;
        }
        Ok(0)
    }

    pub(crate) fn sys_sigpending(&mut self, c: usize, set: u64) -> Result<i64, i64> {
        let t = self.comps[c].linux.as_ref().unwrap();
        let pend = (t.sig.pending | t.p.borrow().sig.pending) & t.sig.mask;
        self.uwrite(c, set, &pend.to_le_bytes()).map(|_| 0)
    }

    pub(crate) fn sys_sigaltstack(&mut self, c: usize, ss: u64, old: u64) -> Result<i64, i64> {
        if old != 0 {
            let t = &self.task(c).sig;
            let mut b = [0u8; 24];
            b[0..8].copy_from_slice(&t.alt_sp.to_le_bytes());
            b[8..12].copy_from_slice(&(if t.alt_size == 0 { SS_DISABLE } else { 0 }).to_le_bytes());
            b[16..24].copy_from_slice(&t.alt_size.to_le_bytes());
            self.uwrite(c, old, &b)?;
        }
        if ss != 0 {
            let b = self.uread(c, ss, 24)?;
            let sp = u64::from_le_bytes(b[0..8].try_into().unwrap());
            let flags = i32::from_le_bytes(b[8..12].try_into().unwrap());
            let size = u64::from_le_bytes(b[16..24].try_into().unwrap());
            let t = &mut self.task(c).sig;
            if flags & SS_DISABLE != 0 {
                t.alt_sp = 0;
                t.alt_size = 0;
            } else {
                if size < 2048 {
                    return Err(ENOMEM);
                }
                t.alt_sp = sp;
                t.alt_size = size;
            }
        }
        Ok(0)
    }

    pub(crate) fn sys_sigsuspend(&mut self, c: usize, set: u64) -> Out {
        // First pass: install the temporary mask; then wait for a signal.
        if self.task(c).sig.saved_mask.is_none() {
            let m = match self.uread_u64(c, set) {
                Ok(m) => m,
                Err(e) => return Out::Ret(-e),
            };
            let t = self.task(c);
            t.sig.saved_mask = Some(t.sig.mask);
            t.sig.mask = m & !UNBLOCKABLE;
        }
        Out::Block
    }

    pub(crate) fn sys_sigtimedwait(&mut self, c: usize, set: u64, info: u64, tmo: u64) -> Out {
        let want = match self.uread_u64(c, set) {
            Ok(s) => s,
            Err(e) => return Out::Ret(-e),
        };
        let pend = {
            let t = self.comps[c].linux.as_ref().unwrap();
            (t.sig.pending | t.p.borrow().sig.pending) & want
        };
        if pend != 0 {
            let si = self.sig_take(c, pend).unwrap();
            self.task(c).wait = Default::default();
            if info != 0 {
                if let Err(e) = self.uwrite(c, info, &si.to_bytes()) {
                    return Out::Ret(-e);
                }
            }
            return Out::Ret(si.signo as i64);
        }
        if tmo != 0 {
            let now = timer::now_ns();
            let deadline = match self.task(c).wait.deadline {
                Some(d) => d,
                None => {
                    let (s, ns) = match (self.uread_u64(c, tmo), self.uread_u64(c, tmo + 8)) {
                        (Ok(s), Ok(ns)) => (s, ns),
                        _ => return Out::Ret(-EFAULT),
                    };
                    let d = now + s * 1_000_000_000 + ns;
                    self.task(c).wait.deadline = Some(d);
                    d
                }
            };
            if now >= deadline {
                self.task(c).wait = Default::default();
                return Out::Ret(-EAGAIN);
            }
        }
        Out::Block
    }

    pub(crate) fn sys_setitimer(&mut self, c: usize, which: u64, new: u64, old: u64) -> Result<i64, i64> {
        if which != 0 {
            return Ok(0); // virtual and profiling timers are not supported
        }
        let now = timer::now_ns();
        let cur = self.proc(c).sig.itimer;
        if old != 0 {
            let (iv, rem) = match cur {
                Some((d, iv)) => (iv, d.saturating_sub(now)),
                None => (0, 0),
            };
            let mut b = [0u8; 32];
            b[0..8].copy_from_slice(&(iv / 1_000_000_000).to_le_bytes());
            b[8..16].copy_from_slice(&((iv % 1_000_000_000) / 1000).to_le_bytes());
            b[16..24].copy_from_slice(&(rem / 1_000_000_000).to_le_bytes());
            b[24..32].copy_from_slice(&((rem % 1_000_000_000) / 1000).to_le_bytes());
            self.uwrite(c, old, &b)?;
        }
        if new != 0 {
            let b = self.uread(c, new, 32)?;
            let g = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
            let iv = g(0) * 1_000_000_000 + g(1) * 1000;
            let val = g(2) * 1_000_000_000 + g(3) * 1000;
            self.proc(c).sig.itimer = if val == 0 { None } else { Some((now + val, iv)) };
        }
        Ok(0)
    }
}
