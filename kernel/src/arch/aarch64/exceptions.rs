//! Exception entry from the vector table.

use crate::kernel;

/// Saved register state of a component (layout shared with `entry.rs`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    pub x: [u64; 31],
    pub sp_el0: u64,
    pub elr: u64,
    pub spsr: u64,
}

const _: () = assert!(core::mem::size_of::<TrapFrame>() == 272);

impl TrapFrame {
    pub const fn zeroed() -> Self {
        TrapFrame { x: [0; 31], sp_el0: 0, elr: 0, spsr: 0 }
    }
}

/// Vector kinds: 4 groups (EL1 with SP0, EL1 with SPx, EL0 AArch64,
/// EL0 AArch32) of 4 (sync, IRQ, FIQ, SError).
const KIND_NAMES: [&str; 4] = ["sync", "irq", "fiq", "serror"];

pub fn init() {
    extern "C" {
        static __vectors: u8;
    }
    // SAFETY: installs the vector table defined in entry.rs.
    unsafe {
        let v = &__vectors as *const u8 as u64;
        core::arch::asm!("msr vbar_el1, {}", "isb", in(reg) v, options(nostack));
    }
}

#[no_mangle]
extern "C" fn exception_entry(frame: &mut TrapFrame, kind: u64) {
    crate::smp::lock();
    match kind {
        8 => kernel::on_user_sync(frame),
        9 => kernel::on_user_irq(frame),
        _ => {
            let group = ["EL1/SP0", "EL1/SPx", "EL0/A64", "EL0/A32"][(kind as usize / 4) % 4];
            panic!(
                "unexpected {} exception from {}: esr={:#x} elr={:#x} far={:#x}",
                KIND_NAMES[kind as usize % 4],
                group,
                super::read_esr(),
                frame.elr,
                super::read_far()
            );
        }
    }
    crate::smp::unlock();
}
