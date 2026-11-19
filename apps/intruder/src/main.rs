//! A deliberately misbehaving component. It holds only a send capability for
//! the console and tries to exceed it; then it touches kernel memory, which
//! must fault and get it stopped (and, per its restart policy, restarted).

#![no_std]
#![no_main]

use flare_rt::abi::{BootInfo, Error, Rights, MSG_WORDS};
use flare_rt::{console, println, sys};

flare_rt::entry!(main);

/// The kernel image on the QEMU virt reference board, in the upper half
/// (TTBR1). The page is mapped, but only for the kernel (EL1).
const KERNEL_TEXT: usize = 0xffff_ff80_4008_0000;

fn report<T: core::fmt::Debug>(what: &str, r: Result<T, Error>) {
    match r {
        Err(e) => println!("intruder: {} -> denied ({})", what, e),
        Ok(v) => println!("intruder: {} -> ALLOWED {:?} (UNEXPECTED)", what, v),
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    println!("intruder: incarnation {} probing authority boundaries", bi.incarnation);
    let Some(console) = console::slot() else { return 1 };
    report("write kernel log via the console capability", sys::debug_write(console, b"x"));
    report("send on empty slot 9", sys::send(9, 0, [0; MSG_WORDS]));
    report("derive RECV from SEND-only console cap", sys::cap_derive(console, Rights::RECV, 0));
    report("bind an interrupt without an irq cap", sys::irq_bind(5, console, 0));
    report("reply without a pending call", sys::reply(0, [0; MSG_WORDS]));
    println!("intruder: reading kernel memory at {:#x}", KERNEL_TEXT);
    // SAFETY: none — this is the attack. The kernel must stop us here.
    let v = unsafe { core::ptr::read_volatile(KERNEL_TEXT as *const u64) };
    println!("intruder: read {:#x} -- this line must never print", v);
    0
}
