//! Kernel log.
//!
//! Until a component holding a `klog:read` capability binds itself as the
//! log reader, output goes straight to the early console. Afterwards it is
//! buffered here and the reader is notified, so the user-space console
//! driver is the only writer to the UART and output never interleaves.

use crate::console;
use crate::global::Global;
use core::fmt::{self, Write};

const SIZE: usize = 16 * 1024;

pub struct Klog {
    buf: [u8; SIZE],
    head: usize,
    len: usize,
    dropped: usize,
    reader_bound: bool,
    dirty: bool,
}

static KLOG: Global<Klog> =
    Global::new(Klog { buf: [0; SIZE], head: 0, len: 0, dropped: 0, reader_bound: false, dirty: false });

fn klog() -> &'static mut Klog {
    // SAFETY: the log is a leaf: no function here calls back into code that
    // takes a second reference.
    unsafe { KLOG.get() }
}

pub fn write(bytes: &[u8]) {
    let k = klog();
    if !k.reader_bound {
        console::write_bytes(bytes);
        return;
    }
    // A screen keeps showing the log when a driver (a UART) reads it too.
    if console::is_screen() {
        console::write_bytes(bytes);
    }
    for &b in bytes {
        if k.len == SIZE {
            k.dropped += 1;
            continue;
        }
        k.buf[(k.head + k.len) % SIZE] = b;
        k.len += 1;
    }
    k.dirty = true;
}

/// Copy up to `out.len()` bytes without consuming them.
pub fn peek(out: &mut [u8]) -> usize {
    let k = klog();
    let n = out.len().min(k.len);
    for (i, o) in out[..n].iter_mut().enumerate() {
        *o = k.buf[(k.head + i) % SIZE];
    }
    n
}

pub fn consume(n: usize) {
    let k = klog();
    let n = n.min(k.len);
    k.head = (k.head + n) % SIZE;
    k.len -= n;
}

pub fn is_empty() -> bool {
    klog().len == 0
}

/// Returns whether new output arrived since the last call.
pub fn take_dirty() -> bool {
    core::mem::replace(&mut klog().dirty, false)
}

/// Switch between buffered (reader bound) and direct output. Unbinding
/// flushes whatever the reader had not drained yet.
pub fn set_reader_bound(bound: bool) {
    let k = klog();
    k.reader_bound = bound;
    if !bound {
        while k.len > 0 {
            let start = k.head;
            let n = k.len.min(SIZE - start);
            console::write_bytes(&k.buf[start..start + n]);
            k.head = (k.head + n) % SIZE;
            k.len -= n;
        }
        if k.dropped > 0 {
            let d = core::mem::replace(&mut k.dropped, 0);
            let _ = Writer.write_fmt(format_args!("[klog: {} bytes dropped]\n", d));
        }
        k.dirty = false;
    }
}

pub struct Writer;

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write(s.as_bytes());
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let _ = Writer.write_fmt(args);
}

#[allow(unused_macros)]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::klog::_print(format_args!($($arg)*)) };
}

macro_rules! kprintln {
    () => { $crate::klog::_print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::klog::_print(format_args!("{}\n", format_args!($($arg)*))) };
}
