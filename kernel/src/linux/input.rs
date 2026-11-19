//! `/dev/input/eventN`: evdev devices fed by user-space input drivers.
//!
//! A driver holding an `Input` capability describes its device once (name,
//! ids, capability bitmaps, absolute axes) and then reports events; every
//! open of the device node receives them as `struct input_event`, with
//! the standard evdev ioctls (EVIOCGBIT, EVIOCGABS, EVIOCGNAME, ...).

use super::abi::*;
use super::fs::{FileKind, EPOLLIN};
use super::Kernel;
use crate::arch::timer;
use alloc::collections::VecDeque;
use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::RefCell;
use flare_abi::proto::input::{Describe, Event};

/// One open of an input device.
pub struct EvdevFile {
    pub device: usize,
    queue: VecDeque<[u8; 24]>,
    clock_monotonic: bool,
}

pub type EvdevRef = Rc<RefCell<EvdevFile>>;

pub struct InputDev {
    desc: Option<Describe>,
    abs_value: [i32; 64],
    keys_down: [u8; 96],
    opens: Vec<Weak<RefCell<EvdevFile>>>,
    pub events: u64,
}

impl InputDev {
    pub fn new() -> InputDev {
        InputDev { desc: None, abs_value: [0; 64], keys_down: [0; 96], opens: Vec::new(), events: 0 }
    }
}

const MAX_QUEUE: usize = 1024;

impl Kernel {
    pub(crate) fn evdev_open(&mut self, device: usize) -> Option<FileKind> {
        let f = Rc::new(RefCell::new(EvdevFile { device, queue: VecDeque::new(), clock_monotonic: false }));
        self.lx().inputs.get_mut(device)?.opens.push(Rc::downgrade(&f));
        Some(FileKind::Evdev(f))
    }

    /// INPUT_REPORT from a driver: op 0 describe, op 1 events.
    pub(crate) fn linux_input_report(&mut self, device: usize, op: u64, bytes: &[u8]) -> bool {
        let Some(lx) = self.linux.as_mut() else { return false };
        let epoch = lx.epoch_ns();
        let Some(dev) = lx.inputs.get_mut(device) else { return false };
        match op {
            0 if bytes.len() == core::mem::size_of::<Describe>() => {
                // SAFETY: Describe is plain old data of exactly this size.
                let d = unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const Describe) };
                if dev.desc.is_none() {
                    let n = d.name.iter().position(|&b| b == 0).unwrap_or(d.name.len());
                    kprintln!(
                        "linux: input device event{}: {}",
                        device,
                        core::str::from_utf8(&d.name[..n]).unwrap_or("?")
                    );
                }
                dev.desc = Some(d);
                true
            }
            1 if bytes.len().is_multiple_of(8) => {
                let now = timer::now_ns();
                dev.opens.retain(|w| w.strong_count() > 0);
                for ev in bytes.chunks_exact(8) {
                    // SAFETY: Event is plain old data of 8 bytes.
                    let e = unsafe { core::ptr::read_unaligned(ev.as_ptr() as *const Event) };
                    dev.events += 1;
                    match e.kind {
                        3 if (e.code as usize) < 64 => dev.abs_value[e.code as usize] = e.value,
                        1 if (e.code as usize) < 96 * 8 => {
                            let (byte, bit) = (e.code as usize / 8, e.code % 8);
                            if e.value != 0 {
                                dev.keys_down[byte] |= 1 << bit;
                            } else {
                                dev.keys_down[byte] &= !(1 << bit);
                            }
                        }
                        _ => {}
                    }
                    for w in &dev.opens {
                        let Some(f) = w.upgrade() else { continue };
                        let mut f = f.borrow_mut();
                        let t = if f.clock_monotonic { now } else { now + epoch } / 1000;
                        let mut rec = [0u8; 24];
                        rec[0..8].copy_from_slice(&(t / 1_000_000).to_le_bytes());
                        rec[8..16].copy_from_slice(&(t % 1_000_000).to_le_bytes());
                        rec[16..18].copy_from_slice(&e.kind.to_le_bytes());
                        rec[18..20].copy_from_slice(&e.code.to_le_bytes());
                        rec[20..24].copy_from_slice(&e.value.to_le_bytes());
                        if f.queue.len() < MAX_QUEUE {
                            f.queue.push_back(rec);
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn evdev_poll(&self, f: &EvdevRef) -> u32 {
        if f.borrow().queue.is_empty() {
            0
        } else {
            EPOLLIN
        }
    }

    pub(crate) fn evdev_read(&mut self, f: &EvdevRef, max: usize) -> Result<Option<Vec<u8>>, i64> {
        if max < 24 {
            return Err(EINVAL);
        }
        let mut f = f.borrow_mut();
        if f.queue.is_empty() {
            return Ok(None);
        }
        let mut out = Vec::new();
        while out.len() + 24 <= max {
            match f.queue.pop_front() {
                Some(r) => out.extend_from_slice(&r),
                None => break,
            }
        }
        Ok(Some(out))
    }

    pub(crate) fn evdev_ioctl(&mut self, c: usize, f: &EvdevRef, req: u64, arg: u64) -> Result<i64, i64> {
        if (req >> 8) & 0xff != 0x45 {
            return Err(ENOTTY);
        }
        let nr = (req & 0xff) as usize;
        let size = ((req >> 16) & 0x3fff) as usize;
        let device = f.borrow().device;
        let (d, abs_value, keys_down) = {
            let dev = &self.lx().inputs[device];
            (dev.desc.ok_or(ENODEV)?, dev.abs_value, dev.keys_down)
        };
        let put = |k: &mut Kernel, data: &[u8]| -> Result<i64, i64> {
            let n = data.len().min(size);
            k.uwrite(c, arg, &data[..n]).map(|_| n as i64)
        };
        match nr {
            0x01 => put(self, &0x0001_0001i32.to_le_bytes()), // EVIOCGVERSION
            0x02 => {
                // EVIOCGID
                let mut b = [0u8; 8];
                for (i, v) in [d.bustype, d.vendor, d.product, d.version].iter().enumerate() {
                    b[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
                }
                put(self, &b)
            }
            0x03 => put(self, &[0u8; 8]), // EVIOCGREP
            0x06 => {
                // EVIOCGNAME
                let n = d.name.iter().position(|&b| b == 0).unwrap_or(63);
                let mut v = d.name[..n].to_vec();
                v.push(0);
                put(self, &v)
            }
            0x07 => put(self, b"flare/input0\0"), // EVIOCGPHYS
            0x08 => put(self, b"\0"),             // EVIOCGUNIQ
            0x09 => put(self, &d.prop_bits),      // EVIOCGPROP
            0x18 => put(self, &keys_down),        // EVIOCGKEY
            0x19..=0x1b => put(self, &[0u8; 8]),  // LED, SND, SW state
            0x20..=0x3f => {
                // EVIOCGBIT(ev)
                let bits: Vec<u8> = match nr - 0x20 {
                    0 => d.ev_bits.to_le_bytes().to_vec(),
                    1 => d.key_bits.to_vec(),
                    2 => d.rel_bits.to_vec(),
                    3 => d.abs_bits.to_vec(),
                    _ => Vec::new(),
                };
                let mut v = bits;
                v.resize(size, 0);
                put(self, &v)
            }
            0x40..=0x7f => {
                // EVIOCGABS(axis)
                let axis = nr - 0x40;
                let a = d.abs[axis];
                let mut b = [0u8; 24];
                for (i, v) in [abs_value[axis], a.min, a.max, a.fuzz, a.flat, a.res].iter().enumerate() {
                    b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                put(self, &b)
            }
            0x80 | 0x81 | 0x84 => Ok(0), // force feedback: no effects
            0x90 | 0x91 => Ok(0),        // EVIOCGRAB, EVIOCREVOKE
            0xa0 => {
                // EVIOCSCLOCKID
                let id = self.uread(c, arg, 4)?;
                f.borrow_mut().clock_monotonic = id[0] == 1;
                Ok(0)
            }
            0xc0..=0xff => Ok(0), // EVIOCSABS
            _ => Err(EINVAL),
        }
    }
}
