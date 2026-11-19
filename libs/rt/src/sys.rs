//! Raw system calls. See `flare_abi::syscall` for the contract.

use core::arch::asm;
use flare_abi::{status, syscall as nr, Error, Message, Rights, MSG_WORDS};

/// A capability slot number.
pub type Slot = u64;

#[inline(always)]
fn raw(n: u64, a: [u64; 8]) -> [u64; 9] {
    let (r0, r1, r2, r3, r4, r5, r6, r7, r8);
    // SAFETY: `svc` traps to the kernel, which only touches the registers
    // listed here and validates every argument.
    unsafe {
        asm!(
            "svc #0",
            inout("x0") a[0] => r0, inout("x1") a[1] => r1, inout("x2") a[2] => r2,
            inout("x3") a[3] => r3, inout("x4") a[4] => r4, inout("x5") a[5] => r5,
            inout("x6") a[6] => r6, inout("x7") a[7] => r7, inout("x8") n => r8,
            options(nostack),
        );
    }
    [r0, r1, r2, r3, r4, r5, r6, r7, r8]
}

fn message(r: &[u64; 9]) -> Message {
    Message { badge: r[1], label: r[2], words: [r[3], r[4], r[5], r[6], r[7], r[8]] }
}

pub fn debug_write(klog: Slot, bytes: &[u8]) -> Result<(), Error> {
    status(raw(nr::DEBUG_WRITE, [klog, bytes.as_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0, 0])[0])
}

pub fn yield_now() {
    raw(nr::YIELD, [0; 8]);
}

pub fn send(ep: Slot, label: u64, w: [u64; MSG_WORDS]) -> Result<(), Error> {
    status(raw(nr::SEND, [ep, label, w[0], w[1], w[2], w[3], w[4], w[5]])[0])
}

pub fn call(ep: Slot, label: u64, w: [u64; MSG_WORDS]) -> Result<Message, Error> {
    let r = raw(nr::CALL, [ep, label, w[0], w[1], w[2], w[3], w[4], w[5]]);
    status(r[0]).map(|_| message(&r))
}

/// `call`, retried when the server dies or drops the call (it is being
/// restarted or replaced): the endpoint survives the server, so the retry
/// reaches its next incarnation. Requests must be idempotent.
pub fn call_retry(ep: Slot, label: u64, w: [u64; MSG_WORDS]) -> Result<Message, Error> {
    let mut tries = 0;
    loop {
        match call(ep, label, w) {
            Err(Error::PeerDied | Error::NoReply) if tries < 1000 => {
                tries += 1;
                yield_now();
            }
            r => return r,
        }
    }
}

pub fn recv(ep: Slot) -> Result<Message, Error> {
    let r = raw(nr::RECV, [ep, 0, 0, 0, 0, 0, 0, 0]);
    status(r[0]).map(|_| message(&r))
}

pub fn reply(label: u64, w: [u64; MSG_WORDS]) -> Result<(), Error> {
    status(raw(nr::REPLY, [label, w[0], w[1], w[2], w[3], w[4], w[5], 0])[0])
}

/// Derive a capability with `rights` (a subset of the source's) into a free
/// slot. `badge` 0 keeps the source's badge.
pub fn cap_derive(slot: Slot, rights: Rights, badge: u64) -> Result<Slot, Error> {
    let r = raw(nr::CAP_DERIVE, [slot, rights.0 as u64, badge, 0, 0, 0, 0, 0]);
    status(r[0]).map(|_| r[1])
}

pub fn cap_revoke(slot: Slot) -> Result<(), Error> {
    status(raw(nr::CAP_REVOKE, [slot, 0, 0, 0, 0, 0, 0, 0])[0])
}

pub fn cap_delete(slot: Slot) -> Result<(), Error> {
    status(raw(nr::CAP_DELETE, [slot, 0, 0, 0, 0, 0, 0, 0])[0])
}

pub fn irq_bind(irq: Slot, ep: Slot, bit: u32) -> Result<(), Error> {
    status(raw(nr::IRQ_BIND, [irq, ep, bit as u64, 0, 0, 0, 0, 0])[0])
}

pub fn irq_ack(irq: Slot) -> Result<(), Error> {
    status(raw(nr::IRQ_ACK, [irq, 0, 0, 0, 0, 0, 0, 0])[0])
}

pub fn klog_bind(klog: Slot, ep: Slot, bit: u32) -> Result<(), Error> {
    status(raw(nr::KLOG_BIND, [klog, ep, bit as u64, 0, 0, 0, 0, 0])[0])
}

pub fn klog_read(klog: Slot, buf: &mut [u8]) -> Result<usize, Error> {
    let r = raw(nr::KLOG_READ, [klog, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0, 0, 0]);
    status(r[0]).map(|_| r[1] as usize)
}

pub fn tty_input(klog: Slot, bytes: &[u8]) -> Result<(), Error> {
    status(raw(nr::TTY_INPUT, [klog, bytes.as_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0, 0])[0])
}

pub fn exit(code: i32) -> ! {
    raw(nr::EXIT, [code as i64 as u64, 0, 0, 0, 0, 0, 0, 0]);
    loop {
        core::hint::spin_loop();
    }
}
