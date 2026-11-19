//! arm64 (AArch64) support: boot, exceptions, MMU, GICv2, generic timer,
//! PSCI.

pub mod exceptions;
pub mod gic;
pub mod mmu;
pub mod psci;
pub mod timer;

mod entry;

pub use exceptions::TrapFrame;

/// Wait for an interrupt with interrupts masked; the pending interrupt is
/// then taken by polling the GIC.
pub fn wait_for_interrupt() {
    // SAFETY: `wfi` has no memory effects.
    unsafe { core::arch::asm!("dsb sy", "wfi", options(nomem, nostack)) };
}

/// Make freshly written instructions visible to instruction fetch.
pub fn sync_icache(addr: u64, len: u64) {
    const LINE: u64 = 64;
    let mut a = addr & !(LINE - 1);
    while a < addr + len {
        // SAFETY: cache maintenance by VA on identity-mapped RAM.
        unsafe { core::arch::asm!("dc cvau, {}", in(reg) a, options(nostack)) };
        a += LINE;
    }
    // SAFETY: barrier and whole-icache invalidate.
    unsafe { core::arch::asm!("dsb ish", "ic iallu", "dsb ish", "isb", options(nostack)) };
}

pub fn read_esr() -> u64 {
    let v: u64;
    // SAFETY: reading a system register.
    unsafe { core::arch::asm!("mrs {}, esr_el1", out(reg) v, options(nomem, nostack)) };
    v
}

pub fn read_far() -> u64 {
    let v: u64;
    // SAFETY: reading a system register.
    unsafe { core::arch::asm!("mrs {}, far_el1", out(reg) v, options(nomem, nostack)) };
    v
}

/// Enter user space for the first time with the given frame.
pub fn enter_first(frame: &'static TrapFrame) -> ! {
    extern "C" {
        fn __enter_first(frame: *const TrapFrame) -> !;
    }
    // SAFETY: `frame` is a valid EL0 frame in static memory (not on the
    // kernel stack, which the trampoline overwrites).
    unsafe { __enter_first(frame) }
}
