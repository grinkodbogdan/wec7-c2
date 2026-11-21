//! `/dev/flarehw`: hardware for a Linux driver host.
//!
//! Flare's own drivers are components holding capabilities. A driver host
//! (Linux's own drivers in a process: LKL, see compat/linux/linuxdrv) is a
//! Linux program and holds no capabilities, so the composition names the
//! devices it may claim (`[linux] hardware = [...]`) and the firmware calls
//! it may make (`[linux] smc = [...]`), and this device gives exactly that:
//!
//! * `ioctl(FLAREHW_INFO)`: the granted devices: name, register windows,
//!   interrupts;
//! * `mmap(offset = physical address)`: a granted register window (Device
//!   memory; Normal non-cacheable for a device described with
//!   `mapping = "memory"`, such as memory shared with firmware), or DMA memory this process allocated (cached; bit 62 of the
//!   offset: non-cacheable). Nothing else is reachable;
//! * `ioctl(FLAREHW_DMA_ALLOC)`: physically contiguous memory, and its
//!   physical address (what the device is told);
//! * `ioctl(FLAREHW_IRQ)`: unmask, mask or set the trigger of a granted
//!   interrupt; `read()` returns the ids of interrupts that fired (4 bytes
//!   each), each masked until unmasked again;
//! * `ioctl(FLAREHW_SMC)`: an allowed SMC Calling Convention call;
//! * `ioctl(FLAREHW_INPUT)`: feed the Linux input devices the composition
//!   gives the host (`[linux] inputs`): name, describe, report events, as
//!   an input driver component does with `INPUT_REPORT`;
//! * `ioctl(FLAREHW_ROOTFS)`: a root filesystem archive the host read from
//!   storage into its DMA memory (crate::ramdisk), added to Linux's files;
//! * `ioctl(FLAREHW_SERIAL)`: carry a serial device the composition gives
//!   the host (`[linux] serials`): bytes for Linux programs reading it, and
//!   what they wrote, as a serial driver component does with `SERIAL_IO`.

use super::abi::*;
use super::Kernel;
use crate::arch::{gic, psci};
use crate::mm::{frame, PAGE_SIZE};
use crate::{boot, platform};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

pub const FLAREHW_INFO: u64 = 0x4600;
pub const FLAREHW_DMA_ALLOC: u64 = 0x4601;
pub const FLAREHW_IRQ: u64 = 0x4602;
pub const FLAREHW_SMC: u64 = 0x4603;
pub const FLAREHW_INPUT: u64 = 0x4604;
pub const FLAREHW_ROOTFS: u64 = 0x4605;
pub const FLAREHW_SERIAL: u64 = 0x4606;
/// mmap offset bit: map DMA memory non-cacheable.
pub const NOCACHE: u64 = 1 << 62;

#[derive(Default)]
pub struct FlareHw {
    pub fired: VecDeque<u32>,
    /// DMA blocks handed out: (physical base, size).
    pub dma: Vec<(u64, u64)>,
    /// Lines the host configured edge-triggered (one bit per INTID).
    pub edge: [u64; 16],
}

/// The devices given to Linux, from the composition.
fn devices() -> impl Iterator<Item = &'static platform::DeviceDesc> {
    boot::LINUX_HARDWARE.iter().filter_map(|n| platform::DEVICES.iter().find(|d| d.name == *n))
}

/// An interrupt line given to Linux.
pub fn granted_irq(intid: u32) -> bool {
    devices().any(|d| d.irqs.contains(&intid))
}

impl Kernel {
    /// What may be mapped at physical `[pa, pa + len)`: the memory attribute,
    /// or `None` when the range is not this host's.
    pub(crate) fn flarehw_window(&self, pa: u64, len: u64) -> Option<u64> {
        let nc = pa & NOCACHE != 0;
        let pa = pa & !NOCACHE;
        let end = pa.checked_add(len)?;
        for d in devices() {
            for r in d.regs {
                let (b, e) = (r.base & !(PAGE_SIZE - 1), (r.base + r.size).div_ceil(PAGE_SIZE) * PAGE_SIZE);
                if pa >= b && end <= e {
                    // Shared memory with firmware (Qualcomm's SMEM) rather
                    // than registers: unaligned accesses must work.
                    if d.str_props.contains(&("mapping", "memory")) {
                        return Some(crate::arch::mmu::ATTR_NORMAL_NC);
                    }
                    return Some(crate::arch::mmu::ATTR_DEVICE);
                }
            }
        }
        let lx = self.linux.as_ref()?;
        if lx.hw.dma.iter().any(|&(b, s)| pa >= b && end <= b + s) {
            return Some(if nc { crate::arch::mmu::ATTR_NORMAL_NC } else { crate::arch::mmu::ATTR_NORMAL });
        }
        None
    }

    /// An interrupt for the driver host: masked until it unmasks it again.
    pub(crate) fn flarehw_irq_fired(&mut self, intid: u32) {
        // A level interrupt stays asserted until Linux's handler has served
        // the device: mask it until the host unmasks it. An edge interrupt
        // fires once per edge and Linux's edge flow never unmasks (it only
        // acknowledges), so masking it would lose every later one (the RPM's
        // replies on Qualcomm SoCs).
        let edge = self.linux.as_ref().is_some_and(|l| l.hw.edge.get(intid as usize / 64).is_some_and(|w| w >> (intid % 64) & 1 != 0));
        if !edge {
            gic::disable(intid);
        }
        if let Some(lx) = self.linux.as_mut() {
            if lx.hw.fired.len() < 1024 {
                lx.hw.fired.push_back(intid);
            }
        }
    }

    pub(crate) fn flarehw_read(&mut self, max: usize) -> Result<Option<Vec<u8>>, i64> {
        if max < 4 {
            return Err(EINVAL);
        }
        let q = &mut self.lx().hw.fired;
        if q.is_empty() {
            return Ok(None);
        }
        let mut out = Vec::new();
        while out.len() + 4 <= max {
            match q.pop_front() {
                Some(i) => out.extend_from_slice(&i.to_le_bytes()),
                None => break,
            }
        }
        Ok(Some(out))
    }

    pub(crate) fn flarehw_readable(&self) -> bool {
        self.linux.as_ref().is_some_and(|l| !l.hw.fired.is_empty())
    }

    pub(crate) fn flarehw_ioctl(&mut self, c: usize, req: u64, arg: u64) -> Result<i64, i64> {
        match req {
            FLAREHW_INFO => {
                // in: u32 index; out: u32 index, u32 nregs, u32 nirqs,
                // name[32], 4 bytes of padding (C alignment), regs[8]
                // (u64 base, u64 size), irqs[8] (u32)
                let idx = u32::from_le_bytes(self.uread(c, arg, 4)?.try_into().unwrap()) as usize;
                let d = devices().nth(idx).ok_or(ENOENT)?;
                let mut b = Vec::with_capacity(12 + 32 + 4 + 128 + 32);
                b.extend_from_slice(&(idx as u32).to_le_bytes());
                b.extend_from_slice(&(d.regs.len().min(8) as u32).to_le_bytes());
                b.extend_from_slice(&(d.irqs.len().min(8) as u32).to_le_bytes());
                let mut name = [0u8; 32];
                let n = d.name.len().min(31);
                name[..n].copy_from_slice(&d.name.as_bytes()[..n]);
                b.extend_from_slice(&name);
                b.extend_from_slice(&[0; 4]);
                for i in 0..8 {
                    let (base, size) = d.regs.get(i).map(|r| (r.base, r.size)).unwrap_or((0, 0));
                    b.extend_from_slice(&base.to_le_bytes());
                    b.extend_from_slice(&size.to_le_bytes());
                }
                for i in 0..8 {
                    b.extend_from_slice(&d.irqs.get(i).copied().unwrap_or(0).to_le_bytes());
                }
                self.uwrite(c, arg, &b).map(|_| 0)
            }
            FLAREHW_DMA_ALLOC => {
                // in: u64 size; out: u64 size, u64 physical address
                let size = u64::from_le_bytes(self.uread(c, arg, 8)?.try_into().unwrap());
                if size == 0 || size > 512 << 20 {
                    return Err(EINVAL);
                }
                let size = size.div_ceil(PAGE_SIZE) * PAGE_SIZE;
                let pa = frame::alloc_contiguous(size / PAGE_SIZE).ok_or(ENOMEM)?;
                self.lx().hw.dma.push((pa, size));
                let mut b = size.to_le_bytes().to_vec();
                b.extend_from_slice(&pa.to_le_bytes());
                self.uwrite(c, arg, &b).map(|_| 0)
            }
            FLAREHW_IRQ => {
                // in: u32 intid, u32 op (0 lines, 1 unmask, 2 mask, 3 trigger), u32 arg (edge)
                let v = self.uread(c, arg, 12)?;
                let w = |i: usize| u32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap());
                let (intid, op, edge) = (w(0), w(1), w(2));
                if op == 0 {
                    return Ok(gic::lines() as i64);
                }
                if !granted_irq(intid) {
                    return Err(EPERM);
                }
                match op {
                    1 => gic::enable(intid),
                    2 => gic::disable(intid),
                    3 => {
                        gic::set_edge(intid, edge != 0);
                        let lx = self.lx();
                        if let Some(w) = lx.hw.edge.get_mut(intid as usize / 64) {
                            if edge != 0 {
                                *w |= 1 << (intid % 64);
                            } else {
                                *w &= !(1 << (intid % 64));
                            }
                        }
                    }
                    _ => return Err(EINVAL),
                }
                Ok(0)
            }
            FLAREHW_INPUT => {
                // in: u32 index (into [linux] inputs), u32 op (0 describe,
                // 1 events, 2 name), u64 ptr, u64 len
                let v = self.uread(c, arg, 24)?;
                let w32 = |i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap());
                let w64 = |i: usize| u64::from_le_bytes(v[i..i + 8].try_into().unwrap());
                let (idx, op, ptr, len) = (w32(0) as usize, w32(4) as u64, w64(8), w64(16));
                let &dev = boot::LINUX_INPUTS.get(idx).ok_or(ENOENT)?;
                if op == 2 {
                    let name = boot::INPUT_DEVICES[dev].as_bytes();
                    let mut out = [0u8; 32];
                    let n = name.len().min(31);
                    out[..n].copy_from_slice(&name[..n]);
                    return self.uwrite(c, ptr, &out[..(len as usize).min(32)]).map(|_| 0);
                }
                if len > 8192 {
                    return Err(EINVAL);
                }
                let bytes = self.uread(c, ptr, len as usize)?;
                if self.linux_input_report(dev, op, &bytes) {
                    Ok(0)
                } else {
                    Err(EINVAL)
                }
            }
            FLAREHW_ROOTFS => {
                // in: u64 physical address, u64 length (inside one of this
                // host's DMA blocks); out: the number of entries added
                let v = self.uread(c, arg, 16)?;
                let pa = u64::from_le_bytes(v[0..8].try_into().unwrap());
                let len = u64::from_le_bytes(v[8..16].try_into().unwrap());
                let end = pa.checked_add(len).ok_or(EINVAL)?;
                if !self.lx().hw.dma.iter().any(|&(b, s)| pa >= b && end <= b + s) {
                    return Err(EPERM);
                }
                let va = crate::arch::mmu::phys_to_virt(pa) as *const u8;
                let a = crate::ramdisk::check(va, len).map_err(|e| {
                    kprintln!("linux: root filesystem from storage: {}", e);
                    EINVAL
                })?;
                let data = crate::ramdisk::materialize(va, a, false).map_err(|e| {
                    kprintln!("linux: root filesystem from storage: {}", e);
                    ENOMEM
                })?;
                match self.lx().vfs.unpack(data) {
                    Ok(n) => {
                        kprintln!("linux: root filesystem from storage: {} entries ({} MiB)", n, data.len() >> 20);
                        Ok(n as i64)
                    }
                    Err(e) => {
                        kprintln!("linux: root filesystem from storage damaged: {}", e);
                        Err(EINVAL)
                    }
                }
            }
            FLAREHW_SERIAL => {
                // in: u32 index (into [linux] serials), u32 op (0 push: bytes
                // for Linux; 1 pull: what Linux wrote), u64 ptr, u64 len;
                // out: bytes moved
                let v = self.uread(c, arg, 24)?;
                let w32 = |i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap());
                let w64 = |i: usize| u64::from_le_bytes(v[i..i + 8].try_into().unwrap());
                let (idx, op, ptr, len) = (w32(0) as usize, w32(4), w64(8), w64(16));
                let &dev = boot::LINUX_SERIALS.get(idx).ok_or(ENOENT)?;
                match op {
                    0 => {
                        let room = crate::kernel::SERIAL_BUF.saturating_sub(self.serials[dev].rx.len());
                        let bytes = self.uread(c, ptr, (len as usize).min(room).min(65536))?;
                        self.serials[dev].rx.extend(bytes.iter());
                        Ok(bytes.len() as i64)
                    }
                    1 => {
                        let n = (len as usize).min(self.serials[dev].tx.len()).min(65536);
                        let bytes: Vec<u8> = self.serials[dev].tx.drain(..n).collect();
                        self.uwrite(c, ptr, &bytes)?;
                        Ok(n as i64)
                    }
                    _ => Err(EINVAL),
                }
            }
            FLAREHW_SMC => {
                let v = self.uread(c, arg, 64)?;
                let mut r = [0u64; 8];
                for (i, x) in r.iter_mut().enumerate() {
                    *x = u64::from_le_bytes(v[i * 8..i * 8 + 8].try_into().unwrap());
                }
                let fid = r[0] as u32;
                if !boot::LINUX_SMC.iter().any(|&(a, b)| fid >= a && fid <= b) {
                    kprintln!("flarehw: SMC function {:#x} refused (not in the composition's [linux] smc)", fid);
                    return Err(EPERM);
                }
                if !psci::smccc(&mut r) {
                    return Err(ENOSYS);
                }
                let mut b = Vec::with_capacity(64);
                for x in r {
                    b.extend_from_slice(&x.to_le_bytes());
                }
                self.uwrite(c, arg, &b).map(|_| 0)
            }
            _ => Err(ENOTTY),
        }
    }
}
