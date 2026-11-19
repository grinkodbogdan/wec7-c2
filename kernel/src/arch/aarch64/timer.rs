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
    rearm();
    // SAFETY: enable the virtual timer, interrupt unmasked.
    unsafe { core::arch::asm!("msr cntv_ctl_el0, {}", in(reg) 1u64, options(nostack)) };
    gic::enable(t.irq);
    kprintln!("timer: {} Hz counter, {} Hz preemption tick on interrupt {}", freq, HZ, t.irq);
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
