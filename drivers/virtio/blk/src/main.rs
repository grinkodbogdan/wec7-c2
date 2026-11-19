//! virtio-blk driver component (virtio-mmio transport, version 2).
//!
//! Serves `flare.block@1.0`. It holds the transport's register window, its
//! interrupt and a DMA region; the virtqueue and request buffers live in
//! the DMA region, whose physical addresses it learns with `DMA_PADDR`.
//! Restarting the driver resets and re-initialises the device; clients'
//! calls in flight fail and are retried against the new incarnation.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use flare_rt::abi::proto::{block, console as bytes};
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY, MSG_WORDS};
use flare_rt::{println, sys};

flare_rt::entry!(main);

// virtio-mmio registers
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
const CONFIG: usize = 0x100;

const S_ACK: u32 = 1;
const S_DRIVER: u32 = 2;
const S_DRIVER_OK: u32 = 4;
const S_FEATURES_OK: u32 = 8;
const F_NEXT: u16 = 1;
const F_WRITE: u16 = 2;

const QSIZE: u16 = 8;
const PAGE: usize = 4096;
// DMA layout: page 0 the virtqueue, page 1 request header and status,
// page 2 the sector buffer.
const DESC: usize = 0;
const AVAIL: usize = 256;
const USED: usize = 512;
const HDR: usize = PAGE;
const STAT: usize = PAGE + 64;
const DATA: usize = 2 * PAGE;

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

    fn init(&mut self) -> Result<u64, &'static str> {
        if self.r(MAGIC) != 0x7472_6976 || self.r(VERSION) != 2 {
            return Err("not a virtio-mmio v2 transport");
        }
        if self.r(DEVICE_ID) != 2 {
            return Err("no block device behind the transport");
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
            return Err("queue too small");
        }
        self.w(QUEUE_NUM, QSIZE as u32);
        for (lo, hi, off) in [(DESC_LO, DESC_HI, DESC), (DRIVER_LO, DRIVER_HI, AVAIL), (DEVICE_LO, DEVICE_HI, USED)] {
            let pa = self.pa(off);
            self.w(lo, pa as u32);
            self.w(hi, (pa >> 32) as u32);
        }
        self.w(QUEUE_READY, 1);
        self.w(STATUS, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
        self.last_used = self.get::<u16>(USED + 2);
        Ok(self.r(CONFIG) as u64 | (self.r(CONFIG + 4) as u64) << 32)
    }

    /// Read one sector into the DMA data buffer.
    fn read_sector(&mut self, sector: u64) -> bool {
        self.put::<u32>(HDR, 0); // VIRTIO_BLK_T_IN
        self.put::<u32>(HDR + 4, 0);
        self.put::<u64>(HDR + 8, sector);
        self.put::<u8>(STAT, 0xff);
        let chain = [(HDR, 16u32, F_NEXT), (DATA, 512, F_NEXT | F_WRITE), (STAT, 1, F_WRITE)];
        for (i, &(off, len, flags)) in chain.iter().enumerate() {
            let d = DESC + i * 16;
            self.put::<u64>(d, self.pa(off));
            self.put::<u32>(d + 8, len);
            self.put::<u16>(d + 12, flags);
            self.put::<u16>(d + 14, (i + 1) as u16);
        }
        let idx = self.get::<u16>(AVAIL + 2);
        self.put::<u16>(AVAIL + 4 + 2 * (idx % QSIZE) as usize, 0);
        fence(Ordering::SeqCst);
        self.put::<u16>(AVAIL + 2, idx.wrapping_add(1));
        fence(Ordering::SeqCst);
        self.w(QUEUE_NOTIFY, 0);
        // Wait for the device (it also interrupts; polling with yields keeps
        // the server simple and single-threaded).
        let mut spins = 0u32;
        while self.get::<u16>(USED + 2) == self.last_used {
            sys::yield_now();
            spins += 1;
            if spins > 1_000_000 {
                return false;
            }
        }
        fence(Ordering::SeqCst);
        self.last_used = self.last_used.wrapping_add(1);
        self.get::<u8>(STAT) == 0
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    let find = |n: &str, k: CapKind| bi.find(n).filter(|c| c.kind() == k);
    let (Some(regs), Some(irq), Some(dma), Some(ep)) = (
        find("regs", CapKind::Mmio),
        find("irq", CapKind::Irq),
        find("dma", CapKind::Dma),
        find("blk", CapKind::Endpoint),
    ) else {
        return 2;
    };
    let mut dev = Dev { regs: regs.vaddr as usize, dma: dma.vaddr as usize, dma_slot: dma.slot(), last_used: 0 };
    let _ = sys::irq_bind(irq.slot(), ep.slot(), 0);
    let capacity = match dev.init() {
        Ok(c) => c,
        Err(e) => {
            println!("virtio-blk: {}", e);
            return 3;
        }
    };
    println!(
        "virtio-blk: {} sectors ({} KiB), incarnation {}, user-space driver with DMA",
        capacity,
        capacity / 2,
        bi.incarnation
    );
    let mut cached: Option<u64> = None;
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 4 };
        if msg.label == LABEL_NOTIFY {
            let s = dev.r(INT_STATUS);
            dev.w(INT_ACK, s);
            let _ = sys::irq_ack(irq.slot());
            continue;
        }
        let mut w = [0u64; MSG_WORDS];
        let label = match msg.label {
            block::INFO => {
                w[0] = capacity;
                w[1] = bi.incarnation as u64;
                0
            }
            block::READ => {
                let (sector, off) = (msg.words[0], msg.words[1] as usize);
                if sector >= capacity || off >= block::SECTOR {
                    u64::MAX
                } else if cached != Some(sector) && !dev.read_sector(sector) {
                    cached = None;
                    u64::MAX
                } else {
                    cached = Some(sector);
                    let n = (block::SECTOR - off).min(bytes::MAX_CHUNK);
                    let mut buf = [0u8; bytes::MAX_CHUNK];
                    for (i, b) in buf[..n].iter_mut().enumerate() {
                        *b = dev.get::<u8>(DATA + off + i);
                    }
                    w = bytes::pack(&buf[..n]);
                    n as u64
                }
            }
            _ => u64::MAX,
        };
        let _ = sys::reply(label, w);
    }
}
