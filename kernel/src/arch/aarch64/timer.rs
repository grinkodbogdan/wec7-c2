//! Preemption tick from the EL1 virtual timer.

use super::gic;
use crate::global::Global;
use crate::platform;

pub const HZ: u64 = 100;

struct Timer {
    interval: u64,
    irq: u32,
}

static TIMER: Global<Timer> = Global::new(Timer { interval: 0, irq: 0 });

pub fn init() {
    let dev = platform::find("timer", &["arm,armv8-timer"]).expect("no arm,armv8-timer in the hardware description");
    let freq: u64;
    // SAFETY: reading the counter frequency.
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack)) };
    // SAFETY: boot-time, single reference.
    let t = unsafe { TIMER.get() };
    t.interval = freq / HZ;
    t.irq = dev.irqs[0];
    init_cpu();
    kprintln!("timer: {} Hz counter, {} Hz preemption tick on interrupt {}", freq, HZ, t.irq);
}

/// Start this CPU's preemption tick (the timer and its PPI are per CPU).
pub fn init_cpu() {
    // EL0 may read the virtual counter (CNTVCT_EL0), as on Linux; nothing
    // else (the boot loader's value is not relied on).
    // SAFETY: timer access control for EL0 only.
    unsafe { core::arch::asm!("msr cntkctl_el1, {}", "isb", in(reg) 1u64 << 1, options(nostack)) };
    rearm();
    // SAFETY: enable the virtual timer, interrupt unmasked.
    unsafe { core::arch::asm!("msr cntv_ctl_el0, {}", in(reg) 1u64, options(nostack)) };
    gic::enable(irq());
}

/// Nanoseconds since boot, from the virtual counter.
pub fn now_ns() -> u64 {
    let (cnt, freq): (u64, u64);
    // SAFETY: reading the counter and its frequency.
    unsafe {
        core::arch::asm!("isb", "mrs {}, cntvct_el0", out(reg) cnt, options(nomem, nostack));
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack));
    }
    ((cnt as u128 * 1_000_000_000) / freq.max(1) as u128) as u64
}

pub fn irq() -> u32 {
    // SAFETY: read-only after init.
    unsafe { TIMER.get().irq }
}

pub fn rearm() {
    // SAFETY: reprogram the timer's down-counter.
    unsafe {
        let interval = TIMER.get().interval;
        core::arch::asm!("msr cntv_tval_el0, {}", in(reg) interval, options(nostack));
    }
}
