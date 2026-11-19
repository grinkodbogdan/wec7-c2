//! QEMU ramfb display driver component.
//!
//! ramfb scans out a framebuffer in guest RAM, configured through the
//! fw_cfg device's `etc/ramfb` file. This driver holds the fw_cfg register
//! window, a little DMA memory for fw_cfg commands, and the `display`
//! shared memory: it points the panel at that memory and publishes the
//! mode in its `flare.display@1.0` header, which the Linux personality's
//! DRM device (/dev/dri/card0) reads. Restarting the driver takes the
//! panel offline and brings it back: a display hotplug.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use flare_rt::abi::proto::display as dp;
use flare_rt::abi::{BootInfo, CapKind, MSG_WORDS};
use flare_rt::{println, sys};

flare_rt::entry!(main);

/// The panel: a portrait phone display.
const WIDTH: u32 = 480;
const HEIGHT: u32 = 854;
const WIDTH_MM: u32 = 50;
const HEIGHT_MM: u32 = 89;
const FB_OFFSET: u32 = 4096;

// fw_cfg MMIO registers (QEMU docs/specs/fw_cfg.rst)
const DATA: usize = 0x00;
const SELECTOR: usize = 0x08;
const DMA_ADDR: usize = 0x10;
const FILE_DIR: u16 = 0x19;
const DMA_ERROR: u32 = 1;
const DMA_SELECT: u32 = 8;
const DMA_WRITE: u32 = 0x10;

struct FwCfg {
    regs: usize,
    dma: usize,
    dma_slot: u64,
}

impl FwCfg {
    fn select(&self, key: u16) {
        // SAFETY: the fw_cfg registers the kernel mapped for us.
        unsafe { write_volatile((self.regs + SELECTOR) as *mut u16, key.to_be()) }
    }
    fn read_bytes(&self, out: &mut [u8]) {
        for b in out {
            // SAFETY: as above; the data register reads one byte at a time.
            *b = unsafe { read_volatile((self.regs + DATA) as *const u8) };
        }
    }

    /// Find a fw_cfg file: (selector, size).
    fn find(&self, name: &str) -> Option<(u16, u32)> {
        self.select(FILE_DIR);
        let mut n = [0u8; 4];
        self.read_bytes(&mut n);
        for _ in 0..u32::from_be_bytes(n) {
            let mut e = [0u8; 64];
            self.read_bytes(&mut e);
            let len = e[8..].iter().position(|&b| b == 0).unwrap_or(56);
            if &e[8..8 + len] == name.as_bytes() {
                return Some((u16::from_be_bytes([e[4], e[5]]), u32::from_be_bytes([e[0], e[1], e[2], e[3]])));
            }
        }
        None
    }

    /// Write `data` to fw_cfg item `key` with the DMA interface.
    fn dma_write(&self, key: u16, data: &[u8]) -> bool {
        let (acc, buf) = (0usize, 64usize);
        // SAFETY: our DMA region (one page).
        unsafe {
            for (i, b) in data.iter().enumerate() {
                write_volatile((self.dma + buf + i) as *mut u8, *b);
            }
            let ctl = ((key as u32) << 16) | DMA_SELECT | DMA_WRITE;
            write_volatile((self.dma + acc) as *mut u32, ctl.to_be());
            write_volatile((self.dma + acc + 4) as *mut u32, (data.len() as u32).to_be());
            let pa = sys::dma_paddr(self.dma_slot, buf as u64).unwrap_or(0);
            write_volatile((self.dma + acc + 8) as *mut u64, pa.to_be());
        }
        fence(Ordering::SeqCst);
        let pa = sys::dma_paddr(self.dma_slot, acc as u64).unwrap_or(0);
        // SAFETY: the DMA address register: high half, then low half starts it.
        unsafe {
            write_volatile((self.regs + DMA_ADDR) as *mut u32, ((pa >> 32) as u32).to_be());
            write_volatile((self.regs + DMA_ADDR + 4) as *mut u32, (pa as u32).to_be());
        }
        for _ in 0..1_000_000 {
            // SAFETY: our DMA region.
            let ctl = u32::from_be(unsafe { read_volatile(self.dma as *const u32) });
            if ctl & !DMA_ERROR == 0 {
                return ctl & DMA_ERROR == 0;
            }
            sys::yield_now();
        }
        false
    }
}

fn header(shm: usize) -> *mut dp::Header {
    shm as *mut dp::Header
}

/// The boot picture: a dark gradient with a soft glow, shown until a
/// compositor takes the display.
fn paint_splash(fb: *mut u32, stride: usize) {
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let (dx, dy) = (x as i32 - WIDTH as i32 / 2, y as i32 - HEIGHT as i32 / 2);
            let d2 = (dx * dx + dy * dy) as u32;
            let glow = 60u32.saturating_sub(d2 / 1200);
            let base = 8 + (y as u32 * 16 / HEIGHT);
            let (r, g, b) = (base + glow / 3, base + glow / 2, base + glow + 6);
            // SAFETY: inside the framebuffer.
            unsafe { write_volatile(fb.add(y * stride / 4 + x), (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)) };
        }
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    let find = |n: &str, k: CapKind| bi.find(n).filter(|c| c.kind() == k);
    let (Some(regs), Some(dma), Some(shm), Some(ep)) = (
        find("fwcfg", CapKind::Mmio),
        find("dma", CapKind::Dma),
        find("fb", CapKind::Dma),
        find("display", CapKind::Endpoint),
    ) else {
        return 2;
    };
    let shm_va = shm.vaddr as usize;
    let stride = WIDTH * 4;
    if (FB_OFFSET + stride * HEIGHT) as u64 > shm.size {
        println!("ramfb: shared memory too small for {}x{}", WIDTH, HEIGHT);
        return 3;
    }
    // Offline while (re)configuring.
    // SAFETY: the header lives at the start of our shared memory.
    let prev_gen = unsafe {
        let h = header(shm_va);
        let g = if read_volatile(&(*h).magic) == dp::MAGIC { read_volatile(&(*h).generation) } else { 0 };
        write_volatile(&mut (*h).state, dp::STATE_OFFLINE);
        g
    };
    let fw = FwCfg { regs: regs.vaddr as usize, dma: dma.vaddr as usize, dma_slot: dma.slot() };
    let Some((key, _)) = fw.find("etc/ramfb") else {
        println!("ramfb: no etc/ramfb in fw_cfg (QEMU needs -device ramfb)");
        return 4;
    };
    if bi.incarnation == 0 {
        paint_splash((shm_va + FB_OFFSET as usize) as *mut u32, stride as usize);
    }
    let fb_pa = match sys::dma_paddr(shm.slot(), FB_OFFSET as u64) {
        Ok(pa) => pa,
        Err(_) => return 5,
    };
    // struct RAMFBCfg: be64 addr, be32 fourcc, flags, width, height, stride
    let mut cfg = [0u8; 28];
    cfg[0..8].copy_from_slice(&fb_pa.to_be_bytes());
    cfg[8..12].copy_from_slice(&dp::XRGB8888.to_be_bytes());
    cfg[16..20].copy_from_slice(&WIDTH.to_be_bytes());
    cfg[20..24].copy_from_slice(&HEIGHT.to_be_bytes());
    cfg[24..28].copy_from_slice(&stride.to_be_bytes());
    if !fw.dma_write(key, &cfg) {
        println!("ramfb: fw_cfg DMA write failed");
        return 6;
    }
    let generation = prev_gen.wrapping_add(1);
    // SAFETY: as above; publish the mode last, then go online.
    unsafe {
        let h = header(shm_va);
        write_volatile(
            h,
            dp::Header {
                magic: dp::MAGIC,
                version: dp::VERSION,
                state: dp::STATE_OFFLINE,
                width: WIDTH,
                height: HEIGHT,
                stride,
                format: dp::XRGB8888,
                generation,
                fb_offset: FB_OFFSET,
                width_mm: WIDTH_MM,
                height_mm: HEIGHT_MM,
                refresh_hz: 60,
            },
        );
        fence(Ordering::SeqCst);
        write_volatile(&mut (*h).state, dp::STATE_ONLINE);
    }
    println!(
        "ramfb: panel {}x{} online (generation {}, incarnation {}), scanning out shared memory at {:#x}",
        WIDTH, HEIGHT, generation, bi.incarnation, fb_pa
    );
    // Serve mode queries; mostly this just keeps the panel alive.
    loop {
        let Ok(_) = sys::recv(ep.slot()) else { return 7 };
        let mut w = [0u64; MSG_WORDS];
        w[0] = WIDTH as u64;
        w[1] = HEIGHT as u64;
        w[2] = generation as u64;
        let _ = sys::reply(0, w);
    }
}
