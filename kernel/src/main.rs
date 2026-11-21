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

/// The boot CPU's clock as measured at boot (MHz), for /proc/cpuinfo.
pub static CPU_MHZ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
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
mod ramdisk;
mod smp;

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn kmain() -> ! {
    // Vectors first: a fault in early boot then panics instead of jumping
    // to whatever the boot loader left in VBAR_EL1.
    console::boot_mapped();
    console::early_mark(2);
    arch::exceptions::init();
    smp::init_boot_cpu();
    console::early_mark(3);
    // SAFETY: called exactly once, first thing, on the boot CPU.
    unsafe { arch::mmu::init() };
    console::mapped();
    console::early_mark(5);
    let heap = heap::init();
    console::early_mark(6);
    console::init();
    kprintln!();
    kprintln!("Flare OS framekernel {} (arm64)", env!("CARGO_PKG_VERSION"));
    // The boot CPU's clock, measured: a chain of dependent additions runs
    // at one per cycle. (A phone's boot loader may leave the cores slow.)
    {
        let n: u64 = 20_000_000;
        let t0 = arch::timer::now_ns();
        let mut x: u64 = 0;
        // SAFETY: register arithmetic only.
        unsafe {
            core::arch::asm!(
                "2:", "add {x}, {x}, #1", "add {x}, {x}, #1", "add {x}, {x}, #1", "add {x}, {x}, #1",
                "subs {n}, {n}, #4", "b.ne 2b",
                x = inout(reg) x, n = inout(reg) n => _, options(nomem, nostack)
            );
        }
        let ns = (arch::timer::now_ns() - t0).max(1);
        let mhz = x * 1000 / ns;
        CPU_MHZ.store(mhz, core::sync::atomic::Ordering::Relaxed);
        kprintln!("cpu: the boot CPU runs at about {} MHz", mhz);
    }
    kprintln!(
        "fhd: board '{}' on soc '{}', {} RAM region(s), {} devices",
        platform::BOARD,
        platform::SOC,
        platform::RAM.len(),
        platform::DEVICES.len()
    );
    let rd = ramdisk::probe();
    mm::frame::init(heap, rd.unwrap_or((0, 0)));
    arch::enable_fp();
    arch::gic::init();
    arch::timer::init();
    smp::lock();
    kernel::start()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if !console::ready() {
        // No text yet: a band of stripes says "panicked in early boot".
        console::boot_mapped();
        for n in 10..15 {
            console::early_mark(n);
        }
    }
    console::emergency(format_args!("\nKERNEL PANIC: {}\n", info));
    if platform::PANIC_HALT {
        // Keep the message on the board's console (a phone's screen).
        loop {
            // SAFETY: idle forever with IRQs masked; FIQs stay on (the
            // secure world's watchdog on Qualcomm SoCs, see arch::wait_for_interrupt).
            unsafe { core::arch::asm!("msr daifset, #2", "msr daifclr, #1", "wfe", options(nomem, nostack)) };
        }
    }
    arch::psci::system_off()
}
