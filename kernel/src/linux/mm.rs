//! Linux process memory: VMAs over a Flare address space, populated on
//! demand.
//!
//! Every mapping is a [`Vma`]; pages appear on first touch (page fault or
//! kernel access). Private anonymous and private file pages are frames owned
//! by the address space (copied on fork). Shared mappings (`MAP_SHARED`,
//! memfd, `/dev/shm`) take their frames from a [`MemObj`] and are mapped
//! unowned, so fork shares them and they outlive any one mapping.

use super::abi::*;
use super::{Kernel, Out};
use crate::arch::mmu::phys_to_virt;
use crate::mm::vspace::{AddressSpace, Prot};
use crate::mm::{frame, page_ceil, page_floor, PAGE_SIZE};
use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

/// Lowest address handed out by mmap without a hint.
const MMAP_FLOOR: u64 = 0x10_0000_0000;
/// Search window for MAP_32BIT.
const LOW_TOP: u64 = 0x8000_0000;
const LOW_FLOOR: u64 = 0x1000_0000;

/// Pages shared between mappings and processes.
pub struct MemObj {
    pages: BTreeMap<u64, u64>,
    pub size: u64,
}

impl MemObj {
    pub fn new(size: u64) -> MemObj {
        MemObj { pages: BTreeMap::new(), size }
    }

    pub fn from_bytes(data: &[u8]) -> MemObj {
        let mut m = MemObj::new(data.len() as u64);
        m.write(0, data);
        m
    }

    /// The frame for page `idx`, allocated (zeroed) on first use.
    pub fn page(&mut self, idx: u64) -> Option<u64> {
        if let Some(&pa) = self.pages.get(&idx) {
            return Some(pa);
        }
        let pa = frame::alloc()?;
        self.pages.insert(idx, pa);
        Some(pa)
    }

    pub fn read(&mut self, off: u64, len: usize) -> Vec<u8> {
        let end = (off + len as u64).min(self.size);
        let mut out = Vec::new();
        let mut pos = off;
        while pos < end {
            let n = (PAGE_SIZE - pos % PAGE_SIZE).min(end - pos);
            match self.pages.get(&(pos / PAGE_SIZE)) {
                // SAFETY: an owned frame reached through the direct map.
                Some(&pa) => out.extend_from_slice(unsafe {
                    core::slice::from_raw_parts((phys_to_virt(pa) + pos % PAGE_SIZE) as *const u8, n as usize)
                }),
                None => out.resize(out.len() + n as usize, 0),
            }
            pos += n;
        }
        out
    }

    pub fn write(&mut self, off: u64, data: &[u8]) -> usize {
        let mut done = 0usize;
        while done < data.len() {
            let pos = off + done as u64;
            let n = ((PAGE_SIZE - pos % PAGE_SIZE) as usize).min(data.len() - done);
            let Some(pa) = self.page(pos / PAGE_SIZE) else { break };
            // SAFETY: as above.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data[done..].as_ptr(),
                    (phys_to_virt(pa) + pos % PAGE_SIZE) as *mut u8,
                    n,
                )
            };
            done += n;
        }
        self.size = self.size.max(off + done as u64);
        done
    }

    pub fn truncate(&mut self, size: u64) {
        let keep = page_ceil(size) / PAGE_SIZE;
        let drop: Vec<u64> = self.pages.range(keep..).map(|(&k, _)| k).collect();
        for k in drop {
            if let Some(pa) = self.pages.remove(&k) {
                frame::free(pa);
            }
        }
        if !size.is_multiple_of(PAGE_SIZE) {
            if let Some(&pa) = self.pages.get(&(size / PAGE_SIZE)) {
                let off = size % PAGE_SIZE;
                // SAFETY: zero the tail of the last page.
                unsafe { core::ptr::write_bytes((phys_to_virt(pa) + off) as *mut u8, 0, (PAGE_SIZE - off) as usize) };
            }
        }
        self.size = size;
    }
}

impl Drop for MemObj {
    fn drop(&mut self) {
        for (_, pa) in core::mem::take(&mut self.pages) {
            frame::free(pa);
        }
    }
}

pub type MemRef = Rc<RefCell<MemObj>>;

#[derive(Clone)]
pub enum Backing {
    Anon,
    /// `off` is the object offset of the VMA's start.
    Shared(MemRef, u64),
    /// Private file mapping: contents copied in on first touch.
    File(Cow<'static, [u8]>, u64),
    /// Private mapping of a page-backed file: pages copied on first touch.
    Private(MemRef, u64),
    /// Physical memory (/dev/flarehw): the VMA's start is physical `base`,
    /// mapped with memory attribute `attr`.
    Phys(u64, u64),
}

#[derive(Clone)]
pub struct Vma {
    pub start: u64,
    pub end: u64,
    pub prot: u64,
    pub backing: Backing,
}

impl Vma {
    fn slice(&self, start: u64, end: u64) -> Vma {
        let delta = start - self.start;
        let backing = match &self.backing {
            Backing::Anon => Backing::Anon,
            Backing::Shared(o, off) => Backing::Shared(o.clone(), off + delta),
            Backing::File(d, off) => Backing::File(d.clone(), off + delta),
            Backing::Private(o, off) => Backing::Private(o.clone(), off + delta),
            Backing::Phys(base, attr) => Backing::Phys(base + delta, *attr),
        };
        Vma { start, end, prot: self.prot, backing }
    }
}

#[derive(Clone, Default)]
pub struct Mm {
    pub vmas: Vec<Vma>,
    pub brk_start: u64,
    pub brk: u64,
}

pub fn prot_of(p: u64) -> Prot {
    Prot { none: p & (PROT_READ | PROT_WRITE | PROT_EXEC) == 0, write: p & PROT_WRITE != 0, exec: p & PROT_EXEC != 0 }
}

/// VMAs are kept sorted by start and never overlap, so lookups are binary
/// searches and allocation is one pass over the gaps.
impl Mm {
    /// Index of the first VMA ending above `va`.
    fn first_ending_above(&self, va: u64) -> usize {
        self.vmas.partition_point(|v| v.end <= va)
    }

    pub fn find(&self, va: u64) -> Option<&Vma> {
        self.vmas.get(self.first_ending_above(va)).filter(|v| v.start <= va)
    }

    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        self.vmas.get(self.first_ending_above(start)).is_some_and(|v| v.start < end)
    }

    pub fn insert(&mut self, v: Vma) {
        let i = self.vmas.partition_point(|x| x.start <= v.start);
        self.vmas.insert(i, v);
    }

    /// Cut `[start, end)` out of the VMA list; returns the removed pieces.
    pub fn carve(&mut self, start: u64, end: u64) -> Vec<Vma> {
        let first = self.first_ending_above(start);
        let mut last = first;
        while last < self.vmas.len() && self.vmas[last].start < end {
            last += 1;
        }
        let mut removed = Vec::new();
        let mut keep = Vec::new();
        for v in self.vmas.drain(first..last) {
            if v.start < start {
                keep.push(v.slice(v.start, start));
            }
            removed.push(v.slice(v.start.max(start), v.end.min(end)));
            if v.end > end {
                keep.push(v.slice(end, v.end));
            }
        }
        self.vmas.splice(first..first, keep);
        removed
    }

    /// Highest free range of `len` bytes below `top` and above `floor`.
    pub fn find_free(&self, len: u64, top: u64, floor: u64) -> Option<u64> {
        // Walk the gaps downwards from `top`.
        let mut hi = top;
        for v in self.vmas.iter().rev() {
            if v.start >= hi {
                continue;
            }
            if v.end < hi && hi - v.end >= len {
                let at = page_floor(hi - len);
                return (at >= floor && at >= v.end).then_some(at);
            }
            hi = v.start;
            if hi < floor.saturating_add(len) {
                return None;
            }
        }
        let at = page_floor(hi.checked_sub(len)?);
        (at >= floor).then_some(at)
    }
}

/// Unmap the pages of `[start, end)` (owned frames are freed).
fn unmap_range(vs: &mut AddressSpace, start: u64, end: u64) {
    let mut va = start;
    while va < end {
        vs.unmap(va);
        va += PAGE_SIZE;
    }
}

impl Kernel {
    /// Bring the page at `va` in, as a fault from user mode or a kernel
    /// access would need it. `false` if the access is not allowed.
    pub(crate) fn linux_populate(&mut self, c: usize, va: u64, write: bool, exec: bool) -> bool {
        let page = page_floor(va);
        let vma = {
            let p = self.proc(c);
            match p.mm.find(va) {
                Some(v) => v.clone(),
                None => return false,
            }
        };
        let prot = prot_of(vma.prot);
        if prot.none || (write && !prot.write) || (exec && !prot.exec) {
            return false;
        }
        let Some(vsr) = self.comps[c].vspace.clone() else { return false };
        let mut vs = vsr.borrow_mut();
        if vs.is_mapped(page) {
            // Mapped but not accessible this way: a real protection fault,
            // unless the VMA's protection was raised since.
            return vs.translate_user(va, write).is_some();
        }
        match &vma.backing {
            Backing::Anon => vs.map_new(page, prot).is_ok(),
            Backing::File(data, off) => {
                let Ok(pa) = vs.map_new(page, Prot::RW) else { return false };
                let foff = off + (page - vma.start);
                if (foff as usize) < data.len() {
                    let n = (data.len() - foff as usize).min(PAGE_SIZE as usize);
                    // SAFETY: a frame just mapped for this page.
                    unsafe {
                        core::ptr::copy_nonoverlapping(data[foff as usize..].as_ptr(), phys_to_virt(pa) as *mut u8, n)
                    };
                }
                if prot.exec {
                    crate::arch::sync_icache(phys_to_virt(pa), PAGE_SIZE);
                }
                vs.protect(page, prot);
                true
            }
            Backing::Private(obj, off) => {
                let Ok(pa) = vs.map_new(page, Prot::RW) else { return false };
                let foff = off + (page - vma.start);
                let data = obj.borrow_mut().read(foff, PAGE_SIZE as usize);
                // SAFETY: a frame just mapped for this page.
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), phys_to_virt(pa) as *mut u8, data.len()) };
                if prot.exec {
                    crate::arch::sync_icache(phys_to_virt(pa), PAGE_SIZE);
                }
                vs.protect(page, prot);
                true
            }
            Backing::Shared(obj, off) => {
                let idx = (off + (page - vma.start)) / PAGE_SIZE;
                let Some(pa) = obj.borrow_mut().page(idx) else { return false };
                vs.map_page(page, pa, prot, false, false).is_ok()
            }
            Backing::Phys(base, attr) => vs.map_phys(page, base + (page - vma.start), prot, *attr).is_ok(),
        }
    }

    /// Make sure `[va, va+len)` is present for a kernel access.
    pub(crate) fn linux_touch(&mut self, c: usize, va: u64, len: usize, write: bool) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = va.checked_add(len as u64) else { return false };
        let mut page = page_floor(va);
        while page < end {
            // (no address space: a thread of a process that is exiting,
            // its memory already gone, e.g. clearing its child tid)
            let ok = {
                let Some(vsr) = self.comps[c].vspace.as_ref() else { return false };
                let vs = vsr.borrow();
                vs.translate_user(page.max(va), write).is_some()
            };
            if !ok && !self.linux_populate(c, page.max(va), write, false) {
                return false;
            }
            page += PAGE_SIZE;
        }
        true
    }

    pub(crate) fn sys_brk(&mut self, c: usize, addr: u64) -> u64 {
        let (start, cur) = {
            let p = self.proc(c);
            (p.mm.brk_start, p.mm.brk)
        };
        if addr < start || addr >= super::STACK_LIMIT {
            return cur;
        }
        let (old_end, new_end) = (page_ceil(cur), page_ceil(addr));
        let mut p = self.proc(c);
        if new_end > old_end {
            if p.mm.overlaps(old_end, new_end) {
                return cur;
            }
            p.mm.insert(Vma { start: old_end, end: new_end, prot: PROT_READ | PROT_WRITE, backing: Backing::Anon });
        } else if new_end < old_end {
            p.mm.carve(new_end, old_end);
            drop(p);
            unmap_range(&mut self.comps[c].vspace.as_ref().unwrap().borrow_mut(), new_end, old_end);
            p = self.proc(c);
        }
        p.mm.brk = addr;
        addr
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sys_mmap(
        &mut self,
        c: usize,
        addr: u64,
        len: u64,
        prot: u64,
        flags: u64,
        fd: u64,
        off: u64,
    ) -> Result<i64, i64> {
        if len == 0 || !off.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        let len = page_ceil(len);
        let shared = flags & 3 == MAP_SHARED || flags & 3 == 3;
        let mut binder = None;
        let mut drm = None;
        let backing = if flags & MAP_ANONYMOUS != 0 {
            if shared {
                Backing::Shared(Rc::new(RefCell::new(MemObj::new(len))), 0)
            } else {
                Backing::Anon
            }
        } else {
            let file = self.proc(c).file(fd)?;
            let f = file.borrow();
            match (&f.kind, shared) {
                (super::fs::FileKind::MemFd(o), true) => Backing::Shared(o.clone(), off),
                (super::fs::FileKind::MemFd(o), false) => {
                    let data = o.borrow_mut().read(off, len as usize);
                    Backing::File(Cow::Owned(data), 0)
                }
                (super::fs::FileKind::Reg(n), true) => Backing::Shared(n.borrow_mut().data.pages(), off),
                (super::fs::FileKind::Reg(n), false) => match &n.borrow().data {
                    super::vfs::Data::Static(s) => Backing::File(Cow::Borrowed(s), off),
                    super::vfs::Data::Pages(o) => Backing::Private(o.clone(), off),
                },
                (super::fs::FileKind::Mem(d), _) => Backing::File(Cow::Owned(d.clone()), off),
                (super::fs::FileKind::Zero, _) => Backing::Anon,
                (super::fs::FileKind::Binder(b), _) => {
                    binder = Some(b.clone());
                    Backing::Anon
                }
                (super::fs::FileKind::Drm(d), _) => {
                    drm = Some(d.clone());
                    Backing::Anon
                }
                (super::fs::FileKind::FlareHw, _) => {
                    drop(f);
                    let attr = self.flarehw_window(off, len).ok_or(EPERM)?;
                    Backing::Phys(off & !super::flarehw::NOCACHE, attr)
                }
                _ => return Err(EACCES),
            }
        };
        let backing = match drm {
            Some(d) => {
                let (obj, base) = self.drm_mmap(&d, off).ok_or(EINVAL)?;
                Backing::Shared(obj, base)
            }
            None => backing,
        };
        let mut p = self.proc(c);
        let va = if flags & MAP_FIXED != 0 {
            if !addr.is_multiple_of(PAGE_SIZE) || addr < USER_MIN_ADDR || addr + len > super::STACK_TOP {
                return Err(EINVAL);
            }
            addr
        } else if flags & MAP_FIXED_NOREPLACE != 0 {
            if !addr.is_multiple_of(PAGE_SIZE) || addr < USER_MIN_ADDR || addr + len > super::STACK_TOP {
                return Err(EINVAL);
            }
            if p.mm.overlaps(addr, addr + len) {
                return Err(EEXIST);
            }
            addr
        } else if flags & MAP_32BIT != 0 {
            p.mm.find_free(len, LOW_TOP, LOW_FLOOR).ok_or(ENOMEM)?
        } else if addr != 0
            && addr.is_multiple_of(PAGE_SIZE)
            && addr >= USER_MIN_ADDR
            && addr + len <= super::MMAP_TOP
            && !p.mm.overlaps(addr, addr + len)
        {
            addr
        } else {
            p.mm.find_free(len, super::MMAP_TOP, MMAP_FLOOR).ok_or(ENOMEM)?
        };
        let backing = match binder {
            // The binder receive area: the kernel writes it, the process reads it.
            Some(b) => Backing::Shared(b.borrow_mut().map(va, len), 0),
            None => backing,
        };
        p.mm.carve(va, va + len);
        p.mm.insert(Vma { start: va, end: va + len, prot, backing });
        drop(p);
        unmap_range(&mut self.comps[c].vspace.as_ref().unwrap().borrow_mut(), va, va + len);
        Ok(va as i64)
    }

    pub(crate) fn sys_munmap(&mut self, c: usize, addr: u64, len: u64) -> Result<i64, i64> {
        if !addr.is_multiple_of(PAGE_SIZE) || len == 0 {
            return Err(EINVAL);
        }
        let end = addr.checked_add(page_ceil(len)).ok_or(EINVAL)?;
        self.proc(c).mm.carve(addr, end);
        unmap_range(&mut self.comps[c].vspace.as_ref().unwrap().borrow_mut(), addr, end);
        Ok(0)
    }

    pub(crate) fn sys_mprotect(&mut self, c: usize, addr: u64, len: u64, prot: u64) -> Result<i64, i64> {
        if !addr.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        let end = addr.checked_add(page_ceil(len)).ok_or(EINVAL)?;
        let mut p = self.proc(c);
        // The whole range must be mapped.
        let mut pos = addr;
        while pos < end {
            match p.mm.find(pos) {
                Some(v) => pos = v.end,
                None => return Err(ENOMEM),
            }
        }
        let pieces = p.mm.carve(addr, end);
        for mut v in pieces {
            v.prot = prot;
            p.mm.insert(v);
        }
        drop(p);
        let pr = prot_of(prot);
        let vsr = self.comps[c].vspace.clone().unwrap();
        let mut vs = vsr.borrow_mut();
        let mut page = addr;
        while page < end {
            if vs.protect(page, pr) && pr.exec {
                if let Some(pa) = vs.lookup(page) {
                    crate::arch::sync_icache(phys_to_virt(page_floor(pa)), PAGE_SIZE);
                }
            }
            page += PAGE_SIZE;
        }
        Ok(0)
    }

    pub(crate) fn sys_mremap(
        &mut self,
        c: usize,
        old: u64,
        old_len: u64,
        new_len: u64,
        flags: u64,
        new_addr: u64,
    ) -> Result<i64, i64> {
        const MAYMOVE: u64 = 1;
        const FIXED: u64 = 2;
        if !old.is_multiple_of(PAGE_SIZE) || new_len == 0 {
            return Err(EINVAL);
        }
        let (old_len, new_len) = (page_ceil(old_len), page_ceil(new_len));
        let vma = {
            let p = self.proc(c);
            let v = p.mm.find(old).ok_or(EFAULT)?;
            if old + old_len > v.end {
                return Err(EFAULT);
            }
            v.clone()
        };
        if new_len <= old_len && flags & FIXED == 0 {
            if new_len < old_len {
                self.sys_munmap(c, old + new_len, old_len - new_len)?;
            }
            return Ok(old as i64);
        }
        // Grow in place if the space after it is free.
        if flags & FIXED == 0 {
            let mut p = self.proc(c);
            let tail = old + old_len;
            if tail == vma.end && !p.mm.overlaps(tail, old + new_len) && old + new_len <= super::STACK_LIMIT {
                let ext = vma.slice(vma.start, vma.end);
                let grown = Vma { end: old + new_len, ..ext };
                p.mm.carve(vma.start, vma.end);
                p.mm.insert(grown);
                return Ok(old as i64);
            }
        }
        if flags & MAYMOVE == 0 {
            return Err(ENOMEM);
        }
        let dest = if flags & FIXED != 0 {
            if !new_addr.is_multiple_of(PAGE_SIZE) {
                return Err(EINVAL);
            }
            self.sys_munmap(c, new_addr, new_len)?;
            new_addr
        } else {
            self.proc(c).mm.find_free(new_len, super::MMAP_TOP, MMAP_FLOOR).ok_or(ENOMEM)?
        };
        // Move the VMA and its present pages.
        let piece = vma.slice(old, old + old_len);
        {
            let mut p = self.proc(c);
            p.mm.carve(old, old + old_len);
            p.mm.insert(Vma { start: dest, end: dest + new_len, ..piece });
        }
        let vsr = self.comps[c].vspace.clone().unwrap();
        let mut vs = vsr.borrow_mut();
        let mut off = 0;
        while off < old_len {
            if let Some(e) = vs.take_entry(old + off) {
                let _ = vs.put_entry(dest + off, e);
            }
            off += PAGE_SIZE;
        }
        Ok(dest as i64)
    }

    pub(crate) fn sys_madvise(&mut self, c: usize, addr: u64, len: u64, advice: u64) -> Result<i64, i64> {
        const DONTNEED: u64 = 4;
        if advice != DONTNEED {
            return Ok(0);
        }
        let end = addr + page_ceil(len);
        let private: Vec<(u64, u64)> = self
            .proc(c)
            .mm
            .vmas
            .iter()
            .filter(|v| v.start < end && addr < v.end && !matches!(v.backing, Backing::Shared(..)))
            .map(|v| (v.start.max(addr), v.end.min(end)))
            .collect();
        let vsr = self.comps[c].vspace.clone().unwrap();
        for (s, e) in private {
            unmap_range(&mut vsr.borrow_mut(), s, e);
        }
        Ok(0)
    }

    /// A user fault: resolve it by populating, or report it as a signal.
    pub(crate) fn linux_fault(&mut self, c: usize, far: u64, esr: u64, pc: u64) {
        let ec = esr >> 26;
        let dfsc = (esr & 0x3f) >> 2;
        let is_data = ec == 0x24;
        let is_inst = ec == 0x20;
        if (is_data || is_inst) && (dfsc == 1 || dfsc == 3) {
            // WnR is also set for cache maintenance (ISS.CM, bit 8: `dc
            // cvau` in __clear_cache after a JIT writes code), which only
            // needs the page readable, as Linux treats it. (QEMU never
            // faults on these; a phone does, on a JIT page not yet touched.)
            let write = is_data && esr & (1 << 6) != 0 && esr & (1 << 8) == 0;
            if self.linux_populate(c, far, write, is_inst) {
                return;
            }
        }
        // MRS of an ID register (op0 3, op1 0, CRn 0): emulated like Linux,
        // as "no optional features" except MIDR_EL1 and MPIDR's CPU-free
        // form, so feature probes fall back instead of dying.
        if ec == 0x18 && esr & 1 == 1 {
            let iss = esr & 0x1ff_ffff;
            let (op0, op2, op1, crn, rt, crm) = (
                (iss >> 20) & 3,
                (iss >> 17) & 7,
                (iss >> 14) & 7,
                (iss >> 10) & 0xf,
                (iss >> 5) & 0x1f,
                (iss >> 1) & 0xf,
            );
            if op0 == 3 && op1 == 0 && crn == 0 {
                let v = match (crm, op2) {
                    // SAFETY: reading an ID register at EL1 has no side effects.
                    (0, 0) => unsafe {
                        let m: u64;
                        core::arch::asm!("mrs {}, midr_el1", out(reg) m);
                        m
                    },
                    (0, 5) => 1 << 31, // MPIDR_EL1: RES1, affinity 0
                    _ => 0,
                };
                let f = &mut self.comps[c].frame;
                if rt != 31 {
                    f.x[rt as usize] = v;
                }
                f.elr = pc + 4;
                return;
            }
        }
        let (sig, code) = match ec {
            0x20 | 0x24 => {
                let mapped = self.proc(c).mm.find(far).is_some();
                (SIGSEGV, if mapped { 2 } else { 1 }) // SEGV_ACCERR / SEGV_MAPERR
            }
            0x22 | 0x26 => (7, 1), // SIGBUS, BUS_ADRALN
            0x3c => (5, 1),        // brk: SIGTRAP
            _ => (4, 1),           // SIGILL, ILL_ILLOPC
        };
        self.linux_force_signal(c, sig, code, far, esr, pc);
    }
}

pub const USER_MIN_ADDR: u64 = PAGE_SIZE;

impl Kernel {
    pub(crate) fn mmap_dispatch(&mut self, c: usize, a: [u64; 6]) -> Out {
        match self.sys_mmap(c, a[0], a[1], a[2], a[3], a[4], a[5]) {
            Ok(v) => Out::Ret(v),
            Err(e) => Out::Ret(-e),
        }
    }
}
