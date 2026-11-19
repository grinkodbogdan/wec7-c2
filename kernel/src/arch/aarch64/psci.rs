//! Power off through PSCI, using the conduit named in the board description.

use crate::platform;

const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;

pub fn system_off() -> ! {
    let method = platform::by_class("firmware")
        .filter(|d| d.compatible.iter().any(|c| c.starts_with("arm,psci")))
        .and_then(|d| d.str_prop("method"));
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
