//! ARM PL011 UART driver component.
//!
//! Runs unprivileged, in its own address space, with exactly:
//! * `uart`     — the PL011 register window (MMIO capability, already mapped),
//! * `uart-irq` — the UART interrupt line,
//! * `console`  — the receive side of the console endpoint it serves,
//! * `klog`     — the kernel log: it prints it (read) and feeds console
//!   input to the kernel's Linux TTY (write).
//!
//! It is the only writer to the UART once it is running, so console output
//! from the kernel and from all clients is serialized here.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use flare_rt::abi::proto::console as proto;
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY, MSG_WORDS};
use flare_rt::sys;

flare_rt::entry!(main);

const DR: usize = 0x000;
const FR: usize = 0x018;
const IMSC: usize = 0x038;
const ICR: usize = 0x044;
const FR_RXFE: u32 = 1 << 4;
const FR_TXFF: u32 = 1 << 5;
const INT_RX: u32 = 1 << 4;
const INT_RT: u32 = 1 << 6;

const IRQ_BIT: u32 = 0;
const KLOG_BIT: u32 = 1;

struct Pl011 {
    base: usize,
}

impl Pl011 {
    fn read(&self, reg: usize) -> u32 {
        // SAFETY: `base` is the PL011 window the kernel mapped for us.
        unsafe { read_volatile((self.base + reg) as *const u32) }
    }
    fn write(&self, reg: usize, v: u32) {
        // SAFETY: as above.
        unsafe { write_volatile((self.base + reg) as *mut u32, v) }
    }
    fn putc(&self, c: u8) {
        while self.read(FR) & FR_TXFF != 0 {}
        self.write(DR, c as u32);
    }
    fn write_bytes(&self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\n' {
                self.putc(b'\r');
            }
            self.putc(b);
        }
    }
    fn getc(&self) -> Option<u8> {
        if self.read(FR) & FR_RXFE != 0 {
            None
        } else {
            Some(self.read(DR) as u8)
        }
    }
}

/// Small ring buffer of received bytes.
struct Input {
    buf: [u8; 256],
    head: usize,
    len: usize,
}

impl Input {
    fn push(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[(self.head + self.len) % self.buf.len()] = b;
            self.len += 1;
        }
    }
    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf[self.head];
        self.head = (self.head + 1) % self.buf.len();
        self.len -= 1;
        Some(b)
    }
}

/// Per-client line buffers (indexed by badge), so output from concurrent
/// clients is never interleaved within a line.
struct Lines {
    buf: [[u8; 160]; 8],
    len: [usize; 8],
}

impl Lines {
    fn write(&mut self, uart: &Pl011, badge: u64, bytes: &[u8]) {
        let i = (badge % 8) as usize;
        for &b in bytes {
            self.buf[i][self.len[i]] = b;
            self.len[i] += 1;
            if b == b'\n' || self.len[i] == self.buf[i].len() {
                uart.write_bytes(&self.buf[i][..self.len[i]]);
                self.len[i] = 0;
            }
        }
    }
}

/// Move received characters into the input buffer for `POLL_INPUT`
/// clients and hand them to the kernel's Linux TTY, which does echo and
/// line editing.
fn drain_rx(uart: &Pl011, input: &mut Input, klog: u64) {
    let mut chunk = [0u8; 64];
    let mut n = 0;
    while let Some(c) = uart.getc() {
        input.push(c);
        chunk[n] = c;
        n += 1;
        if n == chunk.len() {
            let _ = sys::tty_input(klog, &chunk[..n]);
            n = 0;
        }
    }
    if n > 0 {
        let _ = sys::tty_input(klog, &chunk[..n]);
    }
}

fn cap(bi: &BootInfo, name: &str, kind: CapKind) -> u64 {
    match bi.find(name) {
        Some(c) if c.kind() == kind => c.slot(),
        _ => sys::exit(2),
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    let regs = match bi.find("uart") {
        Some(c) if c.kind() == CapKind::Mmio => c,
        _ => return 2,
    };
    let irq = cap(bi, "uart-irq", CapKind::Irq);
    let ep = cap(bi, "console", CapKind::Endpoint);
    let klog = cap(bi, "klog", CapKind::Klog);
    let uart = Pl011 { base: regs.vaddr as usize };

    if sys::irq_bind(irq, ep, IRQ_BIT).is_err() {
        return 3;
    }
    // Clear stale error/modem status, but not RX: input may already be
    // waiting in the FIFO from before we started.
    uart.write(ICR, 0x7ff & !(INT_RX | INT_RT));
    uart.write(IMSC, INT_RX | INT_RT);
    // From here on the kernel log is delivered to us instead of being
    // written to the UART by the kernel.
    if sys::klog_bind(klog, ep, KLOG_BIT).is_err() {
        return 4;
    }
    uart.write_bytes(b"uart-pl011: console service online (incarnation ");
    uart.putc(b'0' + (bi.incarnation % 10) as u8);
    uart.write_bytes(b", user-space driver)\n");

    let mut input = Input { buf: [0; 256], head: 0, len: 0 };
    drain_rx(&uart, &mut input, klog);
    let mut klog_buf = [0u8; 256];
    let mut lines = Lines { buf: [[0; 160]; 8], len: [0; 8] };
    loop {
        let msg = match sys::recv(ep) {
            Ok(m) => m,
            Err(_) => return 5,
        };
        if msg.label == LABEL_NOTIFY {
            let bits = msg.words[0];
            if bits & (1 << KLOG_BIT) != 0 {
                while let Ok(n) = sys::klog_read(klog, &mut klog_buf) {
                    if n == 0 {
                        break;
                    }
                    uart.write_bytes(&klog_buf[..n]);
                }
            }
            if bits & (1 << IRQ_BIT) != 0 {
                drain_rx(&uart, &mut input, klog);
                // Reading DR clears the receive interrupt. Do not write ICR
                // here: that would also clear the status of a character that
                // arrived after the loop, and the level-triggered line would
                // never re-assert.
                let _ = sys::irq_ack(irq);
            }
            continue;
        }
        let (op, len) = proto::split_label(msg.label);
        match op {
            proto::WRITE => {
                let bytes = proto::unpack(&msg.words);
                lines.write(&uart, msg.badge, &bytes[..len.min(proto::MAX_CHUNK)]);
                let _ = sys::reply(0, [0; MSG_WORDS]);
            }
            proto::POLL_INPUT => {
                let mut out = [0u8; proto::MAX_CHUNK];
                let mut n = 0;
                while n < out.len() {
                    match input.pop() {
                        Some(b) => {
                            out[n] = b;
                            n += 1;
                        }
                        None => break,
                    }
                }
                let _ = sys::reply(n as u64, proto::pack(&out[..n]));
            }
            _ => {
                let _ = sys::reply(u64::MAX, [0; MSG_WORDS]);
            }
        }
    }
}
