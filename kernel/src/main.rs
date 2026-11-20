//! The Flare framekernel.
//!
//! A small privileged core: address spaces, frame allocation, scheduling,
//! interrupt dispatch, synchronous IPC and capability enforcement. Drivers
//! and services run as isolated EL0 components that receive only the
//! capabilities their boot composition grants. See `docs/architecture.md`.
//!
//! Concurrency model (M1): one CPU; interrupts are always masked while in
//! the kernel; the kernel never blocks. Every entry runs to completion and
//! then returns to some component (or idles in `wfi`).

#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod klog;

mod arch;
mod boot;
mod console;
mod elf;
mod font;
mod global;
mod heap;
mod kernel;
mod linux;
mod mm;
mod platform;
mod smp;

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn kmain() -> ! {
    smp::init_boot_cpu();
    // SAFETY: called exactly once, first thing, on the boot CPU.
    unsafe { arch::mmu::init() };
    let heap = heap::init();
    console::init();
    kprintln!();
    kprintln!("Flare OS framekernel {} (arm64)", env!("CARGO_PKG_VERSION"));
    kprintln!(
        "fhd: board '{}' on soc '{}', {} RAM region(s), {} devices",
        platform::BOARD,
        platform::SOC,
        platform::RAM.len(),
        platform::DEVICES.len()
    );
    mm::frame::init(heap);
    arch::exceptions::init();
    arch::enable_fp();
    arch::gic::init();
    arch::timer::init();
    smp::lock();
    kernel::start()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    console::emergency(format_args!("\nKERNEL PANIC: {}\n", info));
    if platform::PANIC_HALT {
        // Keep the message on the board's console (a phone's screen).
        loop {
            // SAFETY: idle forever with interrupts masked.
            unsafe { core::arch::asm!("msr daifset, #0xf", "wfe", options(nomem, nostack)) };
        }
    }
    arch::psci::system_off()
}
