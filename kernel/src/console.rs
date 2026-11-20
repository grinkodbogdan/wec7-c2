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
    /// Byte order of one pixel, lowest address first: 0 = blue first
    /// (r8g8b8 / x8r8g8b8 in little-endian memory), 1 = red first.
    red_first: bool,
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

pub fn init() {
    let Some(dev) = platform::CONSOLE.map(|i| &platform::DEVICES[i]) else { return };
    let base = crate::arch::mmu::phys_to_virt(dev.regs[0].base) as usize;
    let backend = if dev.is_compatible("arm,pl011") {
        Backend::Pl011(base)
    } else if dev.is_compatible("simple-framebuffer") {
        let width = dev.int_prop("width").unwrap_or(0) as usize;
        let height = dev.int_prop("height").unwrap_or(0) as usize;
        let format = dev.str_prop("format").unwrap_or("r8g8b8");
        let bpp = match format {
            "a8r8g8b8" | "x8r8g8b8" | "a8b8g8r8" | "x8b8g8r8" => 4,
            "r5g6b5" => 2,
            _ => 3,
        };
        let stride = dev.int_prop("stride").map(|s| s as usize).unwrap_or(width * bpp);
        let scale = dev.int_prop("console-scale").unwrap_or(1).max(1) as usize;
        let (cw, ch) = (font::WIDTH * scale, font::HEIGHT * scale);
        let fb = Fb {
            base,
            stride,
            bpp,
            red_first: format.starts_with("a8b8") || format.starts_with("x8b8") || format == "b8g8r8",
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
        Backend::Fb(fb)
    } else {
        Backend::None
    };
    // SAFETY: single-threaded boot.
    unsafe { *CONSOLE.get() = backend };
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
                let (first, last) = if self.red_first { (r, b) } else { (b, r) };
                out[0] = first;
                out[1] = g;
                out[2] = last;
                if self.bpp == 4 {
                    out[3] = 0xff;
                }
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
        }
    }
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
