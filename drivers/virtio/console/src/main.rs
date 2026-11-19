//! virtio-console driver component (virtio-mmio transport, version 2).
//!
//! Carries the byte stream of one Linux character device (a `Serial`
//! capability): bytes the host sends on the console port go to Linux, and
//! what Linux writes goes to the host. Flare uses it for fdb, the debug
//! bridge; on a phone the same stream would ride a USB gadget function.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
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

const S_ACK: u32 = 1;
const S_DRIVER: u32 = 2;
const S_DRIVER_OK: u32 = 4;
const S_FEATURES_OK: u32 = 8;
const F_WRITE: u16 = 2;

const QSIZE: u16 = 8;
const PAGE: usize = 4096;
// DMA layout: one page per virtqueue, the receive buffers, the send buffer.
const RXQ: usize = 0;
const TXQ: usize = PAGE;
const RXBUF: usize = 2 * PAGE;
const RXBUF_LEN: usize = PAGE / QSIZE as usize;
const TXBUF: usize = 3 * PAGE;
// within a queue page
const DESC: usize = 0;
const AVAIL: usize = 256;
const USED: usize = 512;

const BIT_IRQ: u32 = 0;
const BIT_TX: u32 = 1;

struct Dev {
    regs: usize,
    dma: usize,
    dma_slot: u64,
    rx_used: u16,
    tx_used: u16,
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

    fn setup_queue(&self, index: u32, base: usize) -> Result<(), &'static str> {
        self.w(QUEUE_SEL, index);
        if self.r(QUEUE_NUM_MAX) < QSIZE as u32 {
            return Err("queue too small");
        }
        self.w(QUEUE_NUM, QSIZE as u32);
        for (lo, hi, off) in [(DESC_LO, DESC_HI, DESC), (DRIVER_LO, DRIVER_HI, AVAIL), (DEVICE_LO, DEVICE_HI, USED)] {
            let pa = self.pa(base + off);
            self.w(lo, pa as u32);
            self.w(hi, (pa >> 32) as u32);
        }
        self.w(QUEUE_READY, 1);
        Ok(())
    }

    fn init(&mut self) -> Result<(), &'static str> {
        if self.r(MAGIC) != 0x7472_6976 || self.r(VERSION) != 2 {
            return Err("not a virtio-mmio v2 transport");
        }
        if self.r(DEVICE_ID) != 3 {
            return Err("no console device behind the transport");
        }
        self.w(STATUS, 0);
        self.w(STATUS, S_ACK);
        self.w(STATUS, S_ACK | S_DRIVER);
        self.w(DEV_FEATURES_SEL, 1);
        if self.r(DEV_FEATURES) & 1 == 0 {
            return Err("device lacks VIRTIO_F_VERSION_1");
        }
        // No features beyond VERSION_1: a single port, no multiport control.
        self.w(DRV_FEATURES_SEL, 0);
        self.w(DRV_FEATURES, 0);
        self.w(DRV_FEATURES_SEL, 1);
        self.w(DRV_FEATURES, 1);
        self.w(STATUS, S_ACK | S_DRIVER | S_FEATURES_OK);
        if self.r(STATUS) & S_FEATURES_OK == 0 {
            return Err("features not accepted");
        }
        self.setup_queue(0, RXQ)?;
        self.setup_queue(1, TXQ)?;
        // Every receive descriptor is a buffer the device fills.
        for i in 0..QSIZE as usize {
            let d = RXQ + DESC + i * 16;
            self.put::<u64>(d, self.pa(RXBUF + i * RXBUF_LEN));
            self.put::<u32>(d + 8, RXBUF_LEN as u32);
            self.put::<u16>(d + 12, F_WRITE);
            self.put::<u16>(d + 14, 0);
            self.put::<u16>(RXQ + AVAIL + 4 + 2 * i, i as u16);
        }
        fence(Ordering::SeqCst);
        self.put::<u16>(RXQ + AVAIL + 2, QSIZE);
        self.w(STATUS, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
        self.rx_used = self.get::<u16>(RXQ + USED + 2);
        self.tx_used = self.get::<u16>(TXQ + USED + 2);
        self.w(QUEUE_NOTIFY, 0);
        Ok(())
    }

    /// Hand received bytes to Linux and recycle the buffers.
    fn receive(&mut self, serial: u64) {
        fence(Ordering::SeqCst);
        let used = self.get::<u16>(RXQ + USED + 2);
        let mut any = false;
        while self.rx_used != used {
            let slot = (self.rx_used % QSIZE) as usize;
            let id = self.get::<u32>(RXQ + USED + 4 + slot * 8) as usize % QSIZE as usize;
            let len = (self.get::<u32>(RXQ + USED + 4 + slot * 8 + 4) as usize).min(RXBUF_LEN);
            let mut buf = [0u8; RXBUF_LEN];
            for (k, b) in buf[..len].iter_mut().enumerate() {
                *b = self.get::<u8>(RXBUF + id * RXBUF_LEN + k);
            }
            let mut done = 0;
            while done < len {
                match sys::serial_push(serial, &buf[done..len]) {
                    Ok(0) => sys::yield_now(), // Linux is not reading yet
                    Ok(n) => done += n,
                    Err(_) => break,
                }
            }
            let idx = self.get::<u16>(RXQ + AVAIL + 2);
            self.put::<u16>(RXQ + AVAIL + 4 + 2 * (idx % QSIZE) as usize, id as u16);
            fence(Ordering::SeqCst);
            self.put::<u16>(RXQ + AVAIL + 2, idx.wrapping_add(1));
            self.rx_used = self.rx_used.wrapping_add(1);
            any = true;
        }
        if any {
            fence(Ordering::SeqCst);
            self.w(QUEUE_NOTIFY, 0);
        }
    }

    /// Send what Linux wrote, one buffer at a time.
    fn transmit(&mut self, serial: u64) {
        loop {
            let mut buf = [0u8; PAGE];
            let n = match sys::serial_pull(serial, &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            for (k, b) in buf[..n].iter().enumerate() {
                self.put::<u8>(TXBUF + k, *b);
            }
            self.put::<u64>(TXQ + DESC, self.pa(TXBUF));
            self.put::<u32>(TXQ + DESC + 8, n as u32);
            self.put::<u16>(TXQ + DESC + 12, 0);
            let idx = self.get::<u16>(TXQ + AVAIL + 2);
            self.put::<u16>(TXQ + AVAIL + 4 + 2 * (idx % QSIZE) as usize, 0);
            fence(Ordering::SeqCst);
            self.put::<u16>(TXQ + AVAIL + 2, idx.wrapping_add(1));
            fence(Ordering::SeqCst);
            self.w(QUEUE_NOTIFY, 1);
            let mut spins = 0u32;
            while self.get::<u16>(TXQ + USED + 2) == self.tx_used {
                sys::yield_now();
                spins += 1;
                if spins > 1_000_000 {
                    return;
                }
            }
            self.tx_used = self.tx_used.wrapping_add(1);
        }
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    let find = |n: &str, k: CapKind| bi.find(n).filter(|c| c.kind() == k);
    let (Some(regs), Some(irq), Some(dma), Some(ep), Some(serial)) = (
        find("regs", CapKind::Mmio),
        find("irq", CapKind::Irq),
        find("dma", CapKind::Dma),
        find("irq-ep", CapKind::Endpoint),
        find("serial", CapKind::Serial),
    ) else {
        return 2;
    };
    let mut dev =
        Dev { regs: regs.vaddr as usize, dma: dma.vaddr as usize, dma_slot: dma.slot(), rx_used: 0, tx_used: 0 };
    if let Err(e) = dev.init() {
        println!("virtio-console: {}", e);
        return 3;
    }
    let _ = sys::irq_bind(irq.slot(), ep.slot(), BIT_IRQ);
    let _ = sys::serial_bind(serial.slot(), ep.slot(), BIT_TX);
    println!("virtio-console: port online (incarnation {}, user-space driver)", bi.incarnation);
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 4 };
        if msg.label != LABEL_NOTIFY {
            let _ = sys::reply(u64::MAX, [0; 6]);
            continue;
        }
        let bits = msg.words[0];
        if bits & (1 << BIT_IRQ) != 0 {
            let s = dev.r(INT_STATUS);
            dev.w(INT_ACK, s);
            dev.receive(serial.slot());
            let _ = sys::irq_ack(irq.slot());
        }
        if bits & (1 << BIT_TX) != 0 {
            dev.transmit(serial.slot());
        }
    }
}
