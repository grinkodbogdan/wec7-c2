//! Qualcomm MDP5 display driver component: a phone's panel as
//! `flare.display@1.0`.
//!
//! The boot loader leaves the panel on (continuous splash): the display
//! engine (MDP5) scans a 24-bit buffer through one source pipe, into a layer
//! mixer, out over DSI. This driver keeps all of that and only re-points
//! the pipe at the `display` shared memory, as 32-bit XRGB8888 pixels, which
//! the Linux personality's DRM device (/dev/dri/card0, /dev/fb0) and the
//! kernel console draw into. After QRT's fb.c (0.27.0: "a 32-bit screen"),
//! which does this on the Xiaomi Mi A1: the pipe's address, stride, format
//! and unpack registers, then the pipe's bit in its CTL's FLUSH register,
//! taken at the next frame. Pipe and CTL offsets are MSM8953's (MDP 1.16,
//! Linux's mdp5_cfg.c msm8x53_config).
//!
//! A command-mode panel shows memory only when told: such a panel is kicked
//! (CTL0 START) every 33 ms. The Mi A1's panels are all video mode.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use flare_rt::abi::proto::display as dp;
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY, MSG_WORDS};
use flare_rt::{println, sys};

flare_rt::entry!(main);

const FB_OFFSET: u32 = 4096;
const KICK_NS: u64 = 33_000_000;

/// The source pipes (VIG0, RGB0, RGB1, DMA0, cursor) with their place in a
/// CTL's LAYER (shift) / LAYER_EXT (bit) registers and their FLUSH bit.
const PIPES: [(usize, i32, u32, u32); 5] =
    [(0x04000, 0, 0, 0), (0x14000, 9, 8, 3), (0x16000, 12, 10, 4), (0x24000, 18, 16, 11), (0x34000, -1, 20, 22)];

fn r32(a: usize) -> u32 {
    // SAFETY: inside the MDP register window the kernel mapped for us.
    unsafe { read_volatile(a as *const u32) }
}
fn w32(a: usize, v: u32) {
    // SAFETY: as above.
    unsafe { write_volatile(a as *mut u32, v) }
}

fn stage(lay: u32, ext: u32, sh: i32, ebit: u32) -> u32 {
    if sh < 0 {
        return ext >> 20 & 0xf; // the cursor: 4 bits in EXT
    }
    (lay >> sh as u32 & 7) | (ext >> ebit & 1) << 3
}

struct Pipe {
    regs: usize,
    width: u32,
    height: u32,
    flush_reg: usize,
    flush_bit: u32,
    /// CTL0 START, for a command-mode panel; 0 for video mode.
    kick: usize,
}

/// The pipe the boot loader's splash scans out, and the CTL that flushes it.
fn find_pipe(mdp: usize, size: usize) -> Option<Pipe> {
    for base in [mdp + 0x1000, mdp] {
        for &(off, sh, ebit, fbit) in &PIPES {
            let p = base + off;
            if p + 0x40 > mdp + size {
                continue;
            }
            let (sz, addr, stride, format) = (r32(p), r32(p + 0x14), r32(p + 0x24) & 0xffff, r32(p + 0x30));
            let (w, h, cpp) = (sz & 0xffff, sz >> 16, ((format >> 9) & 3) + 1);
            if !(320..=4096).contains(&w) || !(320..=4096).contains(&h) || addr == 0 || cpp < 3 || stride < w * cpp {
                continue;
            }
            // Its CTL: the first one whose mixers have this pipe staged.
            let mut flush = None;
            for c in 0..3 {
                let cb = base + 0x1000 + 0x200 * c;
                for lm in 0..2 {
                    let (lay, ext) = (r32(cb + 4 * lm), r32(cb + 0x40 + 4 * lm));
                    if flush.is_none() && stage(lay, ext, sh, ebit) != 0 {
                        flush = Some(cb + 0x18);
                    }
                }
            }
            let Some(flush_reg) = flush else { continue };
            let video = [0x6a000usize, 0x6a800, 0x6b000, 0x6b800]
                .iter()
                .any(|&i| base + i + 4 <= mdp + size && r32(base + i) & 1 != 0);
            return Some(Pipe {
                regs: p,
                width: w,
                height: h,
                flush_reg,
                flush_bit: 1 << fbit,
                kick: if video { 0 } else { base + 0x1000 + 0x1c },
            });
        }
    }
    None
}

/// Write back the cached lines of `[va, va + len)`, so the display engine
/// (which reads memory, not the CPU's caches) sees them.
fn clean(va: usize, len: usize) {
    let mut a = va & !63;
    while a < va + len {
        // SAFETY: cache maintenance on our own mapping (EL0 may: SCTLR.UCI).
        unsafe { core::arch::asm!("dc cvac, {}", in(reg) a, options(nostack)) };
        a += 64;
    }
    // SAFETY: barrier only.
    unsafe { core::arch::asm!("dsb sy", options(nostack)) };
}

fn main(bi: &'static BootInfo) -> i32 {
    let find = |n: &str, k: CapKind| bi.find(n).filter(|c| c.kind() == k);
    let (Some(mdp), Some(shm), Some(ep)) =
        (find("mdp", CapKind::Mmio), find("fb", CapKind::Dma), find("display", CapKind::Endpoint))
    else {
        return 2;
    };
    let Some(pipe) = find_pipe(mdp.vaddr as usize, mdp.size as usize) else {
        println!("mdp5-display: no source pipe scanning out (the boot loader left the panel off?)");
        return 3;
    };
    let (w, h) = (pipe.width, pipe.height);
    // Rows in 64-byte units, as QRT uses (1080 pixels: 4352 bytes).
    let pitch = (w * 4 + 63) & !63;
    if (FB_OFFSET + pitch * h) as u64 > shm.size {
        println!("mdp5-display: shared memory too small for {}x{} ({} bytes needed)", w, h, FB_OFFSET + pitch * h);
        return 4;
    }
    let fb_pa = match sys::dma_paddr(shm.slot(), FB_OFFSET as u64) {
        Ok(pa) if pa + (pitch * h) as u64 <= 1 << 32 => pa,
        _ => {
            println!("mdp5-display: the framebuffer must be below 4 GiB (the pipe's address register is 32 bits)");
            return 5;
        }
    };
    let shm_va = shm.vaddr as usize;
    let hdr = shm_va as *mut dp::Header;
    // SAFETY: the header lives at the start of our shared memory.
    let prev_gen = unsafe {
        let g = if read_volatile(&(*hdr).magic) == dp::MAGIC { read_volatile(&(*hdr).generation) } else { 0 };
        write_volatile(&mut (*hdr).state, dp::STATE_OFFLINE);
        g
    };
    let fb_va = shm_va + FB_OFFSET as usize;
    if bi.incarnation == 0 {
        // Black until something draws (the kernel console follows the
        // panel here; a compositor takes it over).
        for i in 0..(pitch * h / 8) as usize {
            // SAFETY: inside the framebuffer.
            unsafe { write_volatile((fb_va as *mut u64).add(i), 0) };
        }
    }
    clean(fb_va, (pitch * h) as usize);
    // Re-point the pipe: rows, 4-byte XRGB8888 pixels, the new address; the
    // CTL takes it at the next frame.
    let p = pipe.regs;
    let f = r32(p + 0x30);
    let f = (f & !(0xc0 | 0x100 | 0x600 | 0x3000)) | 3 << 6 | 3 << 9 | 3 << 12; // 8-bit alpha field, no alpha, 4 bytes, 4 elements
    w32(p + 0x24, (r32(p + 0x24) & 0xffff_0000) | pitch); // SRC_YSTRIDE0
    w32(p + 0x30, f); // SRC_FORMAT
    w32(p + 0x34, 0x0302_0001); // SRC_UNPACK: B, G, R, X
    w32(p + 0x14, fb_pa as u32); // SRC0_ADDR
    fence(Ordering::SeqCst);
    w32(pipe.flush_reg, pipe.flush_bit);
    let generation = prev_gen.wrapping_add(1);
    // SAFETY: as above; publish the mode last, then go online.
    unsafe {
        write_volatile(
            hdr,
            dp::Header {
                magic: dp::MAGIC,
                version: dp::VERSION,
                state: dp::STATE_OFFLINE,
                width: w,
                height: h,
                stride: pitch,
                format: dp::XRGB8888,
                generation,
                fb_offset: FB_OFFSET,
                width_mm: 68, // the Mi A1's 5.5" panel
                height_mm: 121,
                refresh_hz: 60,
            },
        );
        fence(Ordering::SeqCst);
        write_volatile(&mut (*hdr).state, dp::STATE_ONLINE);
    }
    clean(shm_va, 64);
    println!(
        "mdp5-display: panel {}x{} ({} mode) now scans 32-bit pixels at {:#x} (pipe {:#x}, generation {})",
        w,
        h,
        if pipe.kick != 0 { "command" } else { "video" },
        fb_pa,
        p - mdp.vaddr as usize,
        generation
    );
    if pipe.kick != 0 {
        let _ = sys::notify_after(ep.slot(), 0, KICK_NS);
    }
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 7 };
        if msg.label == LABEL_NOTIFY {
            if pipe.kick != 0 {
                w32(pipe.kick, 1);
                let _ = sys::notify_after(ep.slot(), 0, KICK_NS);
            }
            continue;
        }
        let mut wd = [0u64; MSG_WORDS];
        wd[0] = w as u64;
        wd[1] = h as u64;
        wd[2] = generation as u64;
        let _ = sys::reply(0, wd);
    }
}
