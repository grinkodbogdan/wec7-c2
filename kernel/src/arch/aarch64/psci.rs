//! Power off through PSCI, using the conduit named in the board description.

use crate::platform;

const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_CPU_ON: u64 = 0xc400_0003;

fn method() -> Option<&'static str> {
    platform::by_class("firmware")
        .filter(|d| d.compatible.iter().any(|c| c.starts_with("arm,psci")))
        .and_then(|d| d.str_prop("method"))
}

/// Start CPU `mpidr` at physical address `entry` with `x0 = ctx`.
pub fn cpu_on(mpidr: u64, entry: u64, ctx: u64) -> i64 {
    let mut r: u64 = PSCI_CPU_ON;
    // SAFETY: a firmware call.
    unsafe {
        match method() {
            Some("hvc") => {
                core::arch::asm!("hvc #0", inout("x0") r, in("x1") mpidr, in("x2") entry, in("x3") ctx, options(nostack))
            }
            Some("smc") => {
                core::arch::asm!("smc #0", inout("x0") r, in("x1") mpidr, in("x2") entry, in("x3") ctx, options(nostack))
            }
            _ => return -1,
        }
    }
    r as i64
}

/// A firmware call (SMC Calling Convention) through the board's conduit:
/// x0..x7 in `r`, x0..x3 back. `false` if the board has no secure monitor
/// conduit to call.
pub fn smccc(r: &mut [u64; 8]) -> bool {
    let m = method();
    // SAFETY: a firmware call; the caller decides which function ids are allowed.
    unsafe {
        match m {
            Some("smc") => core::arch::asm!("smc #0", inout("x0") r[0], inout("x1") r[1], inout("x2") r[2], inout("x3") r[3],
                inout("x4") r[4], inout("x5") r[5], inout("x6") r[6], inout("x7") r[7], options(nostack)),
            Some("hvc") => core::arch::asm!("hvc #0", inout("x0") r[0], inout("x1") r[1], inout("x2") r[2], inout("x3") r[3],
                inout("x4") r[4], inout("x5") r[5], inout("x6") r[6], inout("x7") r[7], options(nostack)),
            _ => return false,
        }
    }
    true
}

pub fn system_off() -> ! {
    let method = method();
    // SAFETY: firmware call that does not return on success.
    unsafe {
        match method {
            Some("hvc") => core::arch::asm!("hvc #0", in("x0") PSCI_SYSTEM_OFF, options(nostack)),
            Some("smc") => core::arch::asm!("smc #0", in("x0") PSCI_SYSTEM_OFF, options(nostack)),
            _ => {}
        }
    }
    loop {
        // SAFETY: idle forever.
        unsafe { core::arch::asm!("wfe", options(nomem, nostack)) };
    }
}
