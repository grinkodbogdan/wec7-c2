//! Qualcomm MSM UARTDM driver component: the console service on a phone's
//! debug UART (the Xiaomi Mi A1's, on test pads), at the 115200 baud the boot
//! loader set up.
//!
//! Capabilities: `uart` (the register window), `console` (the endpoint it
//! serves) and `klog` (it prints the kernel log). Output only: UARTDM receive
//! (stale timers, packed words) is not implemented yet. A screen console keeps
//! showing the log too (the kernel mirrors it).
//!
//! Transmit as Linux's msm_serial does it, after QRT's src/arch/arm64/main.c:
//! wait for the transmitter, reset TX_READY, write how many characters follow
//! (NCF_TX), then up to four per word into the FIFO (TF). Every wait is
//! bounded: with its clock off (Linux's I2C driver can gate the BLSP clock
//! the UART shares) the UART must not hang the driver. An access that faults
//! instead only kills this component, which the supervisor restarts.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use flare_rt::abi::proto::console as proto;
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY, MSG_WORDS};
use flare_rt::sys;

flare_rt::entry!(main);

const SR: usize = 0x08; // status: bit 2 TX_READY (FIFO room), bit 3 TX_EMPTY
const CR: usize = 0x10; // command
const ISR: usize = 0x14; // interrupt status: bit 7 TX_READY
const NCF_TX: usize = 0x40; // characters in the next transfer
const TF: usize = 0x70; // transmit FIFO
const SPIN: u32 = 200_000;
const KLOG_BIT: u32 = 1;
const CLIENTS: usize = 32;

struct Uartdm(usize);

impl Uartdm {
    fn read(&self, r: usize) -> u32 {
        // SAFETY: the UARTDM window the kernel mapped for us.
        unsafe { read_volatile((self.0 + r) as *const u32) }
    }
    fn write(&self, r: usize, v: u32) {
        // SAFETY: as above.
        unsafe { write_volatile((self.0 + r) as *mut u32, v) }
    }
    /// One transfer of up to 64 bytes.
    fn send(&self, b: &[u8]) {
        let mut t = 0;
        while t < SPIN && self.read(SR) & 8 == 0 && self.read(ISR) & 0x80 == 0 {
            t += 1;
        }
        self.write(CR, 3 << 8); // reset TX_READY
        self.write(NCF_TX, b.len() as u32);
        let _ = self.read(NCF_TX);
        for chunk in b.chunks(4) {
            let mut w = 0u32;
            for (j, &c) in chunk.iter().enumerate() {
                w |= (c as u32) << (8 * j);
            }
            let mut t = 0;
            while t < SPIN && self.read(SR) & 4 == 0 {
                t += 1;
            }
            self.write(TF, w);
        }
    }
    /// Text with "\n" as "\r\n", in transfers of up to 64 bytes.
    fn write_bytes(&self, bytes: &[u8]) {
        let mut buf = [0u8; 64];
        let mut n = 0;
        for &b in bytes {
            if b == b'\n' {
                buf[n] = b'\r';
                n += 1;
            }
            buf[n] = b;
            n += 1;
            if n >= buf.len() - 1 {
                self.send(&buf[..n]);
                n = 0;
            }
        }
        if n > 0 {
            self.send(&buf[..n]);
        }
    }
}

/// Per-client line buffers (by badge), so client lines never interleave.
struct Lines {
    buf: [[u8; 160]; CLIENTS],
    len: [usize; CLIENTS],
}

impl Lines {
    fn write(&mut self, uart: &Uartdm, badge: u64, bytes: &[u8]) {
        let i = (badge as usize) % CLIENTS;
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

fn main(bi: &'static BootInfo) -> i32 {
    let (Some(regs), Some(ep), Some(klog)) = (bi.find("uart"), bi.find("console"), bi.find("klog")) else {
        return 2;
    };
    if regs.kind() != CapKind::Mmio || ep.kind() != CapKind::Endpoint || klog.kind() != CapKind::Klog {
        return 2;
    }
    let uart = Uartdm(regs.vaddr as usize);
    if sys::klog_bind(klog.slot(), ep.slot(), KLOG_BIT).is_err() {
        return 4;
    }
    uart.write_bytes(b"uart-msm: console service online (user-space driver)\n");
    let mut klog_buf = [0u8; 256];
    let mut lines = Lines { buf: [[0; 160]; CLIENTS], len: [0; CLIENTS] };
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 5 };
        if msg.label == LABEL_NOTIFY {
            if msg.words[0] & (1 << KLOG_BIT) != 0 {
                while let Ok(n) = sys::klog_read(klog.slot(), &mut klog_buf) {
                    if n == 0 {
                        break;
                    }
                    uart.write_bytes(&klog_buf[..n]);
                }
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
                let _ = sys::reply(0, [0; MSG_WORDS]);
            }
            _ => {
                let _ = sys::reply(u64::MAX, [0; MSG_WORDS]);
            }
        }
    }
}
