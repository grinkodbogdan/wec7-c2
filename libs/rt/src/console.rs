//! Client side of the `flare.console@1.0` service.
//!
//! If the component was granted an endpoint capability named `console`,
//! `print!`/`println!` send to it. Without that grant, output is dropped:
//! there is no ambient stdout.

use crate::sys::{self, Slot};
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};
use flare_abi::proto::console as proto;
use flare_abi::{BootInfo, CapKind, Error};

const NONE: u64 = u64::MAX;
static CONSOLE: AtomicU64 = AtomicU64::new(NONE);

pub(crate) fn init(bi: &BootInfo) {
    if let Some(c) = bi.find("console").filter(|c| c.kind() == CapKind::Endpoint) {
        CONSOLE.store(c.slot(), Ordering::Relaxed);
    }
}

/// The console endpoint slot, if this component has one.
pub fn slot() -> Option<Slot> {
    match CONSOLE.load(Ordering::Relaxed) {
        NONE => None,
        s => Some(s),
    }
}

/// Write bytes through a specific console capability.
pub fn write_to(slot: Slot, bytes: &[u8]) -> Result<(), Error> {
    for chunk in bytes.chunks(proto::MAX_CHUNK) {
        let reply = sys::call(slot, proto::label(proto::WRITE, chunk.len()), proto::pack(chunk))?;
        if reply.label != 0 {
            return Err(Error::InvalidArg);
        }
    }
    Ok(())
}

pub fn write(bytes: &[u8]) -> Result<(), Error> {
    match slot() {
        Some(s) => write_to(s, bytes),
        None => Err(Error::NoCap),
    }
}

/// Fetch whatever input the console has buffered (may be empty).
pub fn poll_input(buf: &mut [u8; proto::MAX_CHUNK]) -> Result<usize, Error> {
    let s = slot().ok_or(Error::NoCap)?;
    let reply = sys::call(s, proto::label(proto::POLL_INPUT, 0), [0; flare_abi::MSG_WORDS])?;
    let n = (reply.label as usize).min(proto::MAX_CHUNK);
    buf[..n].copy_from_slice(&proto::unpack(&reply.words)[..n]);
    Ok(n)
}

/// Line-buffering formatter so each `println!` becomes few, whole messages.
pub struct Writer {
    buf: [u8; 128],
    len: usize,
}

impl Writer {
    pub const fn new() -> Self {
        Writer { buf: [0; 128], len: 0 }
    }
    pub fn flush(&mut self) {
        let _ = write(&self.buf[..self.len]);
        self.len = 0;
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if self.len == self.buf.len() {
                self.flush();
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut w = Writer::new();
    let _ = fmt::write(&mut w, args);
    w.flush();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::console::_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::console::_print(format_args!("{}\n", format_args!($($arg)*))) };
}
