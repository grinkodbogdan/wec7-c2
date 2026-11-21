//! Early boot console on the board's `chosen.console`:
//! * a PL011 UART, polled;
//! * a `simple-framebuffer`: a framebuffer the boot loader left scanning
//!   out (on phones, the splash screen's buffer), drawn as a text console.
//!   This is how a phone without an accessible UART shows its boot log.
//!
//! Used until a user-space console driver binds the kernel log (if the
//! board has one), after a log reader dies, and for panics.

use crate::font;
use crate::global::Global;
use crate::platform;
use core::fmt::{self, Write};
use core::ptr::{read_volatile, write_volatile};

const DR: usize = 0x000;
const FR: usize = 0x018;
const FR_TXFF: u32 = 1 << 5;

enum Backend {
    None,
    Pl011(usize),
    Fb(Fb),
}

struct Fb {
    base: usize,
    stride: usize,
    bpp: usize,
    /// Where red, green and blue go within a pixel (byte offsets).
    byte_of: [usize; 3],
    /// A command-mode panel's CTL START register (written after drawing).
    kick: usize,
    scale: usize,
    cols: usize,
    rows: usize,
    x0: usize,
    y0: usize,
    col: usize,
    row: usize,
    escape: u8,
}

static CONSOLE: Global<Backend> = Global::new(Backend::None);
/// What MDP5 discovery found, reported once the console is up.
static FOUND: Global<Option<(u64, usize, usize, u64, usize, bool)>> = Global::new(None);

/// The framebuffer console for boot marks (physical base, bytes per row,
/// rows; zeros for none), read by entry.rs with the MMU off.
#[no_mangle]
#[used]
static EARLY_FB: [u64; 3] = platform::EARLY_FB;

static MAPPED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Boot marks, for boards whose only output is a screen: before the console
/// draws text, stage `n` fills rows `[48n, 48n + 32)` of the framebuffer
/// white (marks 0 and 1 come from entry.rs). A photo of a stuck boot shows
/// how far it got; the console clears them once it is up.
/// 0 entered, 1 at the linked address (entry.rs, MMU off), 2 MMU on and
/// in Rust, 3 exception vectors, 4 kernel page tables built, 5 switched to
/// them, 6 heap (the console starts next). A panic before the console has
/// text fills rows 480-720 with five stripes.
pub fn early_mark(n: u64) {
    let [base, stride, rows] = EARLY_FB;
    if base == 0 || !MAPPED.load(core::sync::atomic::Ordering::Relaxed) || 48 * n + 32 > rows {
        return;
    }
    let p = crate::arch::mmu::phys_to_virt(base + 48 * n * stride) as *mut u64;
    for i in 0..(32 * stride / 8) as usize {
        // SAFETY: inside the described framebuffer, mapped (non-cacheable) by mmu::init.
        unsafe { core::ptr::write_volatile(p.add(i), u64::MAX) };
    }
}

/// Running on the boot tables, which map the framebuffer when it is in the
/// kernel image's gigabyte (entry.rs).
pub fn boot_mapped() {
    // The kernel image's physical address, from this function's own.
    let kernel = crate::arch::mmu::virt_to_phys(boot_mapped as *const () as usize as u64);
    if EARLY_FB[0] != 0 && EARLY_FB[0] >> 30 == kernel >> 30 {
        MAPPED.store(true, core::sync::atomic::Ordering::Relaxed);
    }
}

/// The direct map (and with it the framebuffer) is up.
pub fn mapped() {
    MAPPED.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// Whether the console can show text.
pub fn ready() -> bool {
    // SAFETY: read of a word-sized discriminant; set once at boot.
    !matches!(unsafe { &*CONSOLE.get() }, Backend::None)
}

pub fn init() {
    let Some(dev) = platform::CONSOLE.map(|i| &platform::DEVICES[i]) else { return };
    let base = crate::arch::mmu::phys_to_virt(dev.regs[0].base) as usize;
    let backend = if dev.is_compatible("arm,pl011") {
        Backend::Pl011(base)
    } else if dev.is_compatible("simple-framebuffer") {
        let mut width = dev.int_prop("width").unwrap_or(0) as usize;
        let mut height = dev.int_prop("height").unwrap_or(0) as usize;
        let format = dev.str_prop("format").unwrap_or("r8g8b8");
        let mut bpp = match format {
            "a8r8g8b8" | "x8r8g8b8" | "a8b8g8r8" | "x8b8g8r8" => 4,
            "r5g6b5" => 2,
            _ => 3,
        };
        let mut stride = dev.int_prop("stride").map(|s| s as usize).unwrap_or(width * bpp);
        let red_first = format.starts_with("a8b8") || format.starts_with("x8b8") || format == "b8g8r8";
        let mut byte_of = if red_first { [0, 1, 2] } else { [2, 1, 0] };
        let mut base = base;
        let mut kick = 0;
        // The boot loader's real setup, where the display engine can say.
        let window = (dev.regs[0].base, dev.regs[0].size);
        let found = mdp5_splash(window);
        if let Some(m) = &found {
            base = crate::arch::mmu::phys_to_virt(m.addr) as usize;
            (width, height, stride, bpp, byte_of, kick) = (m.width, m.height, m.stride, m.bpp, m.byte_of, m.kick);
        }
        // SAFETY: single-threaded boot.
        unsafe { *FOUND.get() = found.map(|m| (m.pipe, m.width, m.height, m.addr, m.bpp, m.video)) };
        let scale = dev.int_prop("console-scale").unwrap_or(1).max(1) as usize;
        let (cw, ch) = (font::WIDTH * scale, font::HEIGHT * scale);
        let fb = Fb {
            base,
            stride,
            bpp,
            byte_of,
            kick,
            scale,
            cols: (width / cw).max(1),
            rows: (height / ch).max(1),
            x0: (width % cw) / 2,
            y0: (height % ch) / 2,
            col: 0,
            row: 0,
            escape: 0,
        };
        // Start from a black screen: the splash image is still there.
        for y in 0..height {
            // SAFETY: inside the framebuffer, mapped write-combining.
            unsafe { core::ptr::write_bytes((base + y * stride) as *mut u8, 0, width * bpp) };
        }
        fb.flush();
        Backend::Fb(fb)
    } else {
        Backend::None
    };
    // SAFETY: single-threaded boot.
    unsafe { *CONSOLE.get() = backend };
    // SAFETY: as above.
    if let Some((pipe, w, h, addr, bpp, video)) = unsafe { *FOUND.get() } {
        kprintln!(
            "console: the boot loader's splash: MDP5 pipe {:#x}, {}x{} at {:#x}, {}-byte pixels, {} mode",
            pipe,
            w,
            h,
            addr,
            bpp,
            if video { "video" } else { "command" }
        );
    }
}

fn putc(base: usize, c: u8) {
    // SAFETY: `base` is the console's MMIO window, identity-mapped as device
    // memory for the kernel.
    unsafe {
        while read_volatile((base + FR) as *const u32) & FR_TXFF != 0 {}
        write_volatile((base + DR) as *mut u32, c as u32);
    }
}

impl Fb {
    fn pixel(&self, on: bool, out: &mut [u8]) {
        // light grey on black; the console has no colours
        let (r, g, b) = if on { (0xe8u8, 0xe8u8, 0xe8u8) } else { (0, 0, 0) };
        match self.bpp {
            2 => {
                let v = ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3);
                out[..2].copy_from_slice(&v.to_le_bytes());
            }
            _ => {
                if self.bpp == 4 {
                    out[3] = 0xff;
                }
                out[self.byte_of[0]] = r;
                out[self.byte_of[1]] = g;
                out[self.byte_of[2]] = b;
            }
        }
    }

    fn draw(&self, col: usize, row: usize, c: u8) {
        let glyph = if (32..127).contains(&c) { &font::GLYPHS[(c - 32) as usize] } else { &font::GLYPHS[0] };
        let s = self.scale;
        let mut line = [0u8; font::WIDTH * 4 * 4];
        let w = font::WIDTH * s * self.bpp;
        let x = self.x0 + col * font::WIDTH * s;
        let y = self.y0 + row * font::HEIGHT * s;
        for (gy, bits) in glyph.iter().enumerate() {
            for gx in 0..font::WIDTH {
                let on = bits >> (font::WIDTH - 1 - gx) & 1 != 0;
                for k in 0..s.min(4) {
                    let o = ((gx * s) + k) * self.bpp;
                    self.pixel(on, &mut line[o..o + self.bpp]);
                }
            }
            for k in 0..s {
                let dst = self.base + (y + gy * s + k) * self.stride + x * self.bpp;
                // SAFETY: inside the framebuffer (cols/rows fit its size).
                unsafe { core::ptr::copy_nonoverlapping(line.as_ptr(), dst as *mut u8, w.min(line.len())) };
            }
        }
    }

    /// A command-mode panel shows memory only when told.
    fn flush(&self) {
        if self.kick != 0 {
            // SAFETY: the MDP's CTL0 START register, mapped as device memory.
            unsafe { write_volatile(self.kick as *mut u32, 1) };
        }
    }

    fn clear_row(&self, row: usize) {
        for c in 0..self.cols {
            self.draw(c, row, b' ');
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        // No scrolling (reading the framebuffer back is slow): wrap to the
        // top, keeping a blank row below the cursor as a marker.
        self.row = (self.row + 1) % self.rows;
        self.clear_row(self.row);
        self.clear_row((self.row + 1) % self.rows);
    }

    fn write(&mut self, c: u8) {
        // Skip terminal escape sequences (ESC [ ... final byte).
        match self.escape {
            1 => {
                self.escape = if c == b'[' { 2 } else { 0 };
                return;
            }
            2 => {
                if (0x40..0x7f).contains(&c) {
                    self.escape = 0;
                }
                return;
            }
            _ => {}
        }
        match c {
            0x1b => self.escape = 1,
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            b'\t' => {
                for _ in 0..(8 - self.col % 8) {
                    self.write(b' ');
                }
            }
            8 => self.col = self.col.saturating_sub(1),
            c if c < 0x20 => {}
            c => {
                if self.col == self.cols {
                    self.newline();
                }
                self.draw(self.col, self.row, if c < 0x80 { c } else { b'?' });
                self.col += 1;
            }
        }
    }
}

/// The console is a screen (drawn by the kernel even when a user-space
/// console driver reads the log).
pub fn is_screen() -> bool {
    // SAFETY: read of a value set at boot.
    matches!(unsafe { CONSOLE.get() }, Backend::Fb(_))
}

pub fn write_bytes(bytes: &[u8]) {
    // SAFETY: the console is set up at boot; after that, callers hold the
    // kernel lock (or are panicking).
    match unsafe { CONSOLE.get() } {
        Backend::None => {}
        Backend::Pl011(base) => {
            for &b in bytes {
                if b == b'\n' {
                    putc(*base, b'\r');
                }
                putc(*base, b);
            }
        }
        Backend::Fb(fb) => {
            // UTF-8 continuation bytes would each show as '?': show one per
            // character instead.
            for &b in bytes {
                if b & 0xc0 != 0x80 {
                    fb.write(b);
                }
            }
            fb.flush();
        }
    }
}

/// What a Qualcomm MDP5 source pipe scans out (the boot loader's splash).
struct Mdp5Splash {
    addr: u64,
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
    byte_of: [usize; 3],
    kick: usize,
    pipe: u64,
    video: bool,
}

/// Find the source pipe the boot loader's continuous splash uses, from the
/// display engine's registers; only a buffer inside `window` (the board's
/// splash memory) is accepted. After QRT's fb.c (pipe offsets from Linux's
/// mdp5_cfg.c, msm8x53_config).
fn mdp5_splash(window: (u64, u64)) -> Option<Mdp5Splash> {
    let mdp = platform::find("device", &["qcom,mdp5"])?;
    let r32 = |pa: u64| -> u32 {
        // SAFETY: inside the MDP's described register window, mapped as device memory.
        unsafe { read_volatile(crate::arch::mmu::phys_to_virt(pa) as *const u32) }
    };
    let (mbase, msize) = (mdp.regs[0].base, mdp.regs[0].size);
    for b in [mbase + 0x1000, mbase] {
        for pipe in [0x04000u64, 0x14000, 0x16000, 0x24000, 0x34000] {
            let p = b + pipe;
            if p + 0x40 > mbase + msize {
                continue;
            }
            let (size, addr, stride, format) = (r32(p), r32(p + 0x14) as u64, (r32(p + 0x24) & 0xffff) as usize, r32(p + 0x30));
            let (w, h, cpp) = ((size & 0xffff) as usize, (size >> 16) as usize, (((format >> 9) & 3) + 1) as usize);
            if !(320..=4096).contains(&w) || !(320..=4096).contains(&h) || addr == 0 || cpp < 3 || stride < w * cpp {
                continue;
            }
            if addr < window.0 || addr + (stride * h) as u64 > window.0 + window.1 {
                continue; // never draw outside the described memory
            }
            // byte i of a pixel holds channel unpack[i]: 0 G, 1 B, 2 R
            let unpack = r32(p + 0x34);
            let mut byte_of = [usize::MAX; 3];
            for i in 0..cpp {
                match (unpack >> (8 * i)) & 3 {
                    2 => byte_of[0] = i,
                    0 => byte_of[1] = i,
                    1 => byte_of[2] = i,
                    _ => {}
                }
            }
            if byte_of.contains(&usize::MAX) {
                byte_of = [2, 1, 0];
            }
            // no interface timing engine running: a command-mode panel
            let video = [0x6a000u64, 0x6a800, 0x6b000, 0x6b800].iter().any(|&i| b + i < mbase + msize && r32(b + i) & 1 != 0);
            let kick = if video { 0 } else { crate::arch::mmu::phys_to_virt(b + 0x1000 + 0x1c) as usize };
            return Some(Mdp5Splash { addr, width: w, height: h, stride, bpp: cpp, byte_of, kick, pipe, video });
        }
    }
    None
}

struct Direct;

impl Write for Direct {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

/// Write straight to the console, bypassing the kernel log (panics).
pub fn emergency(args: fmt::Arguments) {
    let _ = Direct.write_fmt(args);
}
