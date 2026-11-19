//! Early boot console: polled PL011 output on the board's `chosen.console`.
//!
//! Used until a user-space console driver binds the kernel log, after a
//! log reader dies, and for panics.

use crate::global::Global;
use crate::platform;
use core::fmt::{self, Write};
use core::ptr::{read_volatile, write_volatile};

const DR: usize = 0x000;
const FR: usize = 0x018;
const FR_TXFF: u32 = 1 << 5;

static BASE: Global<usize> = Global::new(0);

pub fn init() {
    if let Some(dev) = platform::CONSOLE.map(|i| &platform::DEVICES[i]) {
        if dev.is_compatible("arm,pl011") {
            // SAFETY: single-threaded boot.
            unsafe { *BASE.get() = dev.regs[0].base as usize };
        }
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

pub fn write_bytes(bytes: &[u8]) {
    // SAFETY: read of a word written once at boot.
    let base = unsafe { *BASE.get() };
    if base == 0 {
        return;
    }
    for &b in bytes {
        if b == b'\n' {
            putc(base, b'\r');
        }
        putc(base, b);
    }
}

struct Direct;

impl Write for Direct {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

/// Write straight to the UART, bypassing the kernel log (panics).
pub fn emergency(args: fmt::Arguments) {
    let _ = Direct.write_fmt(args);
}
