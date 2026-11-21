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
    // FIQs stay unmasked while idle: on Qualcomm SoCs the secure world
    // services its watchdog with FIQs, and a CPU sleeping with them masked
    // gets the phone reset (QRT found this on the Xiaomi Mi A1). They never
    // reach non-secure EL1 itself.
    // SAFETY: `wfi` has no memory effects; F is masked again afterwards.
    unsafe { core::arch::asm!("dsb sy", "msr daifclr, #1", "wfi", "msr daifset, #1", options(nomem, nostack)) };
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
    // SAFETY: barrier and whole-icache invalidate, on every CPU (inner
    // shareable): the program may run on another core.
    unsafe { core::arch::asm!("dsb ish", "ic ialluis", "dsb ish", "isb", options(nostack)) };
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

/// Enter user space for the first time on this CPU with the given frame.
pub fn enter_first(frame: &'static TrapFrame, stack_top: u64) -> ! {
    extern "C" {
        fn __enter_first(frame: *const TrapFrame, stack_top: u64) -> !;
    }
    // SAFETY: `frame` is a valid EL0 frame in static memory (not on the
    // kernel stack, which the trampoline overwrites).
    unsafe { __enter_first(frame, stack_top) }
}

/// Top of the boot CPU's kernel stack.
pub fn boot_stack_top() -> u64 {
    extern "C" {
        static __stack_top: u8;
    }
    // SAFETY: only the symbol's address is used.
    unsafe { &__stack_top as *const u8 as u64 }
}

/// User state the kernel never touches itself (it is built soft-float):
/// FP/SIMD registers, their control/status registers and the TLS register.
/// Switched lazily, only when a different component runs.
#[repr(C, align(16))]
#[derive(Clone, Default)]
pub struct ExtState {
    pub v: [u128; 32],
    pub fpcr: u64,
    pub fpsr: u64,
    pub tpidr: u64,
    _pad: u64,
}

core::arch::global_asm!(
    r#"
.arch_extension fp
.arch_extension simd
.global __ext_save
__ext_save:
    stp q0, q1, [x0, #0]
    stp q2, q3, [x0, #32]
    stp q4, q5, [x0, #64]
    stp q6, q7, [x0, #96]
    stp q8, q9, [x0, #128]
    stp q10, q11, [x0, #160]
    stp q12, q13, [x0, #192]
    stp q14, q15, [x0, #224]
    stp q16, q17, [x0, #256]
    stp q18, q19, [x0, #288]
    stp q20, q21, [x0, #320]
    stp q22, q23, [x0, #352]
    stp q24, q25, [x0, #384]
    stp q26, q27, [x0, #416]
    stp q28, q29, [x0, #448]
    stp q30, q31, [x0, #480]
    add x3, x0, #512
    mrs x1, fpcr
    mrs x2, fpsr
    stp x1, x2, [x3]
    mrs x1, tpidr_el0
    str x1, [x3, #16]
    ret
.global __ext_restore
__ext_restore:
    ldp q0, q1, [x0, #0]
    ldp q2, q3, [x0, #32]
    ldp q4, q5, [x0, #64]
    ldp q6, q7, [x0, #96]
    ldp q8, q9, [x0, #128]
    ldp q10, q11, [x0, #160]
    ldp q12, q13, [x0, #192]
    ldp q14, q15, [x0, #224]
    ldp q16, q17, [x0, #256]
    ldp q18, q19, [x0, #288]
    ldp q20, q21, [x0, #320]
    ldp q22, q23, [x0, #352]
    ldp q24, q25, [x0, #384]
    ldp q26, q27, [x0, #416]
    ldp q28, q29, [x0, #448]
    ldp q30, q31, [x0, #480]
    add x3, x0, #512
    ldp x1, x2, [x3]
    msr fpcr, x1
    msr fpsr, x2
    ldr x1, [x3, #16]
    msr tpidr_el0, x1
    ret
"#
);

pub fn ext_save(s: &mut ExtState) {
    extern "C" {
        fn __ext_save(s: *mut ExtState);
    }
    // SAFETY: writes exactly one ExtState.
    unsafe { __ext_save(s) }
}

pub fn ext_restore(s: &ExtState) {
    extern "C" {
        fn __ext_restore(s: *const ExtState);
    }
    // SAFETY: reads exactly one ExtState.
    unsafe { __ext_restore(s) }
}

/// Let EL0 (and the context-switch code) use FP/SIMD.
pub fn enable_fp() {
    // SAFETY: CPACR_EL1.FPEN = 0b11.
    unsafe {
        core::arch::asm!(
            "mrs {t}, cpacr_el1",
            "orr {t}, {t}, #(3 << 20)",
            "msr cpacr_el1, {t}",
            "isb",
            t = out(reg) _,
            options(nostack)
        )
    };
}
