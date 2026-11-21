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

/// The earliest time something waits for (a Linux timeout, a timer file,
/// an alarm): ns since boot, u64::MAX for none. Rebuilt on every pass over
/// the waiters (`reset_wakes`, then each `want_wake`).
static NEXT_WAKE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(u64::MAX);

/// Something waits until `t` (ns since boot): no CPU's timer may sleep past
/// it. Without this, a timeout would wait for the next 100 Hz tick (Linux's
/// drivers in the driver host time everything this way).
pub fn want_wake(t: u64) {
    NEXT_WAKE.fetch_min(t, core::sync::atomic::Ordering::Relaxed);
}

/// Start collecting the earliest wake again (before re-running waiters).
pub fn reset_wakes() {
    NEXT_WAKE.store(u64::MAX, core::sync::atomic::Ordering::Relaxed);
}

/// Each CPU's next preemption tick (counter value).
static NEXT_TICK: [core::sync::atomic::AtomicU64; 8] = [const { core::sync::atomic::AtomicU64::new(0) }; 8];

fn counter() -> u64 {
    let c: u64;
    // SAFETY: reading the virtual counter.
    unsafe { core::arch::asm!("isb", "mrs {}, cntvct_el0", out(reg) c, options(nomem, nostack)) };
    c
}

/// The tick fired: schedule this CPU's next one, then arm.
pub fn rearm() {
    // SAFETY: read-only after init.
    let interval = unsafe { TIMER.get().interval };
    let cpu = crate::smp::cpu().min(7);
    NEXT_TICK[cpu].store(counter() + interval, core::sync::atomic::Ordering::Relaxed);
    arm();
}

/// Program this CPU's timer for its next tick, or the earliest wake if
/// sooner (without moving the tick: preemption stays periodic).
pub fn arm() {
    let cpu = crate::smp::cpu().min(7);
    let tick = NEXT_TICK[cpu].load(core::sync::atomic::Ordering::Relaxed);
    let mut at = tick;
    let wake = NEXT_WAKE.load(core::sync::atomic::Ordering::Relaxed);
    if wake != u64::MAX {
        let freq: u64;
        // SAFETY: reading the counter frequency.
        unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack)) };
        let w = ((wake as u128 * freq as u128).div_ceil(1_000_000_000)) as u64;
        // at least 20 us from now
        at = at.min(w.max(counter() + freq / 50_000));
    }
    // SAFETY: the timer's compare value (absolute).
    unsafe { core::arch::asm!("msr cntv_cval_el0, {}", in(reg) at, options(nostack)) };
}
