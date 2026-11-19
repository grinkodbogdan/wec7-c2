//! virtio-input driver component (virtio-mmio transport, version 2).
//!
//! Reads the device's identity and capabilities from its configuration
//! space, describes it to the Linux personality (it holds an `Input`
//! capability for one /dev/input/eventN), then forwards every event the
//! device posts on its event queue. virtio-input events are evdev events,
//! so they pass through unchanged.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use flare_rt::abi::proto::input::{Abs, Describe, Event};
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY};
use flare_rt::{println, sys};

flare_rt::entry!(main);

const MAGIC: usize = 0x000;
const VERSION: usize = 0x004;
const DEVICE_ID: usize = 0x008;
const DEV_FEATURES: usize = 0x010;
const DEV_FEATURES_SEL: usize = 0x014;
const DRV_FEATURES: usize = 0x020;
const DRV_FEATURES_SEL: usize = 0x024;
const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_READY: usize = 0x044;
const QUEUE_NOTIFY: usize = 0x050;
const INT_STATUS: usize = 0x060;
const INT_ACK: usize = 0x064;
const STATUS: usize = 0x070;
const DESC_LO: usize = 0x080;
const DESC_HI: usize = 0x084;
const DRIVER_LO: usize = 0x090;
const DRIVER_HI: usize = 0x094;
const DEVICE_LO: usize = 0x0a0;
const DEVICE_HI: usize = 0x0a4;
// virtio-input configuration space
const CFG_SELECT: usize = 0x100;
const CFG_SUBSEL: usize = 0x101;
const CFG_SIZE: usize = 0x102;
const CFG_DATA: usize = 0x108;
const ID_NAME: u8 = 0x01;
const ID_DEVIDS: u8 = 0x03;
const PROP_BITS: u8 = 0x10;
const EV_BITS: u8 = 0x11;
const ABS_INFO: u8 = 0x12;

const S_ACK: u32 = 1;
const S_DRIVER: u32 = 2;
const S_DRIVER_OK: u32 = 4;
const S_FEATURES_OK: u32 = 8;
const F_WRITE: u16 = 2;

const QSIZE: u16 = 64;
// DMA layout: descriptors, available ring, used ring, then event buffers.
const DESC: usize = 0;
const AVAIL: usize = 1024;
const USED: usize = 2048;
const BUFS: usize = 4096;

struct Dev {
    regs: usize,
    dma: usize,
    dma_slot: u64,
    last_used: u16,
}

impl Dev {
    fn r(&self, off: usize) -> u32 {
        // SAFETY: the transport registers the kernel mapped for us.
        unsafe { read_volatile((self.regs + off) as *const u32) }
    }
    fn w(&self, off: usize, v: u32) {
        // SAFETY: as above.
        unsafe { write_volatile((self.regs + off) as *mut u32, v) }
    }
    fn r8(&self, off: usize) -> u8 {
        // SAFETY: as above (configuration space is byte-addressable).
        unsafe { read_volatile((self.regs + off) as *const u8) }
    }
    fn w8(&self, off: usize, v: u8) {
        // SAFETY: as above.
        unsafe { write_volatile((self.regs + off) as *mut u8, v) }
    }
    fn pa(&self, off: usize) -> u64 {
        sys::dma_paddr(self.dma_slot, off as u64).unwrap_or(0)
    }
    fn put<T>(&self, off: usize, v: T) {
        // SAFETY: inside our DMA region.
        unsafe { write_volatile((self.dma + off) as *mut T, v) }
    }
    fn get<T: Copy>(&self, off: usize) -> T {
        // SAFETY: inside our DMA region.
        unsafe { read_volatile((self.dma + off) as *const T) }
    }

    /// Read configuration item (select, subsel) into `out`; returns its size.
    fn config(&self, select: u8, subsel: u8, out: &mut [u8]) -> usize {
        self.w8(CFG_SELECT, select);
        self.w8(CFG_SUBSEL, subsel);
        let n = (self.r8(CFG_SIZE) as usize).min(out.len());
        for (i, b) in out[..n].iter_mut().enumerate() {
            *b = self.r8(CFG_DATA + i);
        }
        n
    }

    fn describe(&self) -> Describe {
        let mut d = Describe {
            name: [0; 64],
            bustype: 0x06, // BUS_VIRTUAL
            vendor: 0,
            product: 0,
            version: 1,
            ev_bits: 0,
            key_bits: [0; 96],
            rel_bits: [0; 2],
            abs_bits: [0; 8],
            prop_bits: [0; 4],
            abs: [Abs::default(); 64],
        };
        self.config(ID_NAME, 0, &mut d.name[..63]);
        let mut ids = [0u8; 8];
        if self.config(ID_DEVIDS, 0, &mut ids) == 8 {
            d.bustype = u16::from_le_bytes([ids[0], ids[1]]);
            d.vendor = u16::from_le_bytes([ids[2], ids[3]]);
            d.product = u16::from_le_bytes([ids[4], ids[5]]);
            d.version = u16::from_le_bytes([ids[6], ids[7]]);
        }
        self.config(PROP_BITS, 0, &mut d.prop_bits);
        d.ev_bits = 1; // EV_SYN
        for ev in 1u8..0x20 {
            let mut bits = [0u8; 128];
            let n = self.config(EV_BITS, ev, &mut bits);
            if n == 0 {
                continue;
            }
            d.ev_bits |= 1 << ev;
            match ev {
                1 => d.key_bits.copy_from_slice(&bits[..96]),
                2 => d.rel_bits.copy_from_slice(&bits[..2]),
                3 => d.abs_bits.copy_from_slice(&bits[..8]),
                _ => {}
            }
        }
        for axis in 0..64u8 {
            if d.abs_bits[axis as usize / 8] & (1 << (axis % 8)) == 0 {
                continue;
            }
            let mut a = [0u8; 20];
            if self.config(ABS_INFO, axis, &mut a) >= 8 {
                let v = |i: usize| i32::from_le_bytes([a[i], a[i + 1], a[i + 2], a[i + 3]]);
                d.abs[axis as usize] = Abs { min: v(0), max: v(4), fuzz: v(8), flat: v(12), res: v(16) };
            }
        }
        d
    }

    fn init(&mut self) -> Result<(), &'static str> {
        if self.r(MAGIC) != 0x7472_6976 || self.r(VERSION) != 2 {
            return Err("not a virtio-mmio v2 transport");
        }
        if self.r(DEVICE_ID) != 18 {
            return Err("no input device behind the transport");
        }
        self.w(STATUS, 0);
        self.w(STATUS, S_ACK);
        self.w(STATUS, S_ACK | S_DRIVER);
        self.w(DEV_FEATURES_SEL, 1);
        if self.r(DEV_FEATURES) & 1 == 0 {
            return Err("device lacks VIRTIO_F_VERSION_1");
        }
        self.w(DRV_FEATURES_SEL, 0);
        self.w(DRV_FEATURES, 0);
        self.w(DRV_FEATURES_SEL, 1);
        self.w(DRV_FEATURES, 1);
        self.w(STATUS, S_ACK | S_DRIVER | S_FEATURES_OK);
        if self.r(STATUS) & S_FEATURES_OK == 0 {
            return Err("features not accepted");
        }
        self.w(QUEUE_SEL, 0);
        if self.r(QUEUE_NUM_MAX) < QSIZE as u32 {
            return Err("event queue too small");
        }
        self.w(QUEUE_NUM, QSIZE as u32);
        for (lo, hi, off) in [(DESC_LO, DESC_HI, DESC), (DRIVER_LO, DRIVER_HI, AVAIL), (DEVICE_LO, DEVICE_HI, USED)] {
            let pa = self.pa(off);
            self.w(lo, pa as u32);
            self.w(hi, (pa >> 32) as u32);
        }
        // Every descriptor is an 8-byte event buffer the device fills.
        for i in 0..QSIZE as usize {
            let d = DESC + i * 16;
            self.put::<u64>(d, self.pa(BUFS + i * 8));
            self.put::<u32>(d + 8, 8);
            self.put::<u16>(d + 12, F_WRITE);
            self.put::<u16>(d + 14, 0);
            self.put::<u16>(AVAIL + 4 + 2 * i, i as u16);
        }
        fence(Ordering::SeqCst);
        self.put::<u16>(AVAIL + 2, QSIZE);
        self.w(QUEUE_READY, 1);
        self.w(STATUS, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
        self.last_used = self.get::<u16>(USED + 2);
        self.w(QUEUE_NOTIFY, 0);
        Ok(())
    }

    /// Collect posted events and give their buffers back to the device.
    fn drain(&mut self, out: &mut [Event; 64]) -> usize {
        let mut n = 0;
        fence(Ordering::SeqCst);
        let used = self.get::<u16>(USED + 2);
        while self.last_used != used && n < out.len() {
            let slot = (self.last_used % QSIZE) as usize;
            let id = self.get::<u32>(USED + 4 + slot * 8) as usize % QSIZE as usize;
            let b = BUFS + id * 8;
            out[n] = Event { kind: self.get::<u16>(b), code: self.get::<u16>(b + 2), value: self.get::<i32>(b + 4) };
            n += 1;
            let idx = self.get::<u16>(AVAIL + 2);
            self.put::<u16>(AVAIL + 4 + 2 * (idx % QSIZE) as usize, id as u16);
            fence(Ordering::SeqCst);
            self.put::<u16>(AVAIL + 2, idx.wrapping_add(1));
            self.last_used = self.last_used.wrapping_add(1);
        }
        if n > 0 {
            fence(Ordering::SeqCst);
            self.w(QUEUE_NOTIFY, 0);
        }
        n
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    let find = |n: &str, k: CapKind| bi.find(n).filter(|c| c.kind() == k);
    let (Some(regs), Some(irq), Some(dma), Some(ep), Some(input)) = (
        find("regs", CapKind::Mmio),
        find("irq", CapKind::Irq),
        find("dma", CapKind::Dma),
        find("irq-ep", CapKind::Endpoint),
        find("input", CapKind::Input),
    ) else {
        return 2;
    };
    let mut dev = Dev { regs: regs.vaddr as usize, dma: dma.vaddr as usize, dma_slot: dma.slot(), last_used: 0 };
    if let Err(e) = dev.init() {
        println!("virtio-input: {}", e);
        return 3;
    }
    let d = dev.describe();
    let name_len = d.name.iter().position(|&b| b == 0).unwrap_or(63);
    let name = core::str::from_utf8(&d.name[..name_len]).unwrap_or("?");
    // SAFETY: Describe is plain old data.
    let bytes =
        unsafe { core::slice::from_raw_parts(&d as *const Describe as *const u8, core::mem::size_of::<Describe>()) };
    if sys::input_report(input.slot(), 0, bytes).is_err() {
        println!("virtio-input: the kernel refused the device description");
        return 4;
    }
    println!("virtio-input: '{}' online (incarnation {}, user-space driver)", name, bi.incarnation);
    let _ = sys::irq_bind(irq.slot(), ep.slot(), 0);
    let mut buf = [Event::default(); 64];
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 5 };
        if msg.label != LABEL_NOTIFY {
            let _ = sys::reply(u64::MAX, [0; 6]);
            continue;
        }
        let s = dev.r(INT_STATUS);
        dev.w(INT_ACK, s);
        loop {
            let n = dev.drain(&mut buf);
            if n == 0 {
                break;
            }
            // SAFETY: Event is plain old data.
            let bytes = unsafe { core::slice::from_raw_parts(buf.as_ptr() as *const u8, n * 8) };
            let _ = sys::input_report(input.slot(), 1, bytes);
        }
        let _ = sys::irq_ack(irq.slot());
    }
}
