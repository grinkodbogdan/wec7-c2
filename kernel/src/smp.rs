//! Multicore support.
//!
//! Every CPU runs components and Linux threads in parallel. Kernel code is
//! serialised by one big kernel lock, taken on every exception entry and
//! released just before returning to user mode (the approach QRT's ARM
//! kernel uses): the kernel's data structures stay single-threaded, while
//! user code - where the time goes - runs on all cores. Idle CPUs wait with
//! the lock released and are woken by an inter-processor interrupt when work
//! becomes ready.

use crate::arch::{self, gic, mmu, psci};
use crate::platform;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub const MAX_CPUS: usize = 8;
pub const STACK_SIZE: usize = 64 * 1024;
/// SGI used to wake idle CPUs.
pub const SGI_WAKE: u32 = 1;

static LOCK: AtomicBool = AtomicBool::new(false);
static IDLE: AtomicU64 = AtomicU64::new(0);
static ONLINE: AtomicUsize = AtomicUsize::new(1);

#[repr(C, align(16))]
pub struct Stacks([[u8; STACK_SIZE]; MAX_CPUS]);

/// Kernel stacks of the secondary CPUs (CPU 0 uses the linker's stack).
#[no_mangle]
pub static mut SECONDARY_STACKS: Stacks = Stacks([[0; STACK_SIZE]; MAX_CPUS]);

/// This CPU's index (kept in TPIDR_EL1).
#[inline(always)]
pub fn cpu() -> usize {
    let v: u64;
    // SAFETY: reading a system register.
    unsafe { core::arch::asm!("mrs {}, tpidr_el1", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v as usize
}

fn set_cpu(id: usize) {
    // SAFETY: TPIDR_EL1 is reserved for the CPU index.
    unsafe { core::arch::asm!("msr tpidr_el1, {}", in(reg) id as u64, options(nomem, nostack)) };
}

pub fn online() -> usize {
    ONLINE.load(Ordering::Relaxed)
}

pub fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        while LOCK.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
    }
}

pub fn unlock() {
    LOCK.store(false, Ordering::Release);
}

/// Wait for an interrupt with the kernel lock released.
pub fn idle_wait() {
    let bit = 1u64 << cpu();
    IDLE.fetch_or(bit, Ordering::SeqCst);
    unlock();
    arch::wait_for_interrupt();
    lock();
    IDLE.fetch_and(!bit, Ordering::SeqCst);
}

/// Work became ready: wake one idle CPU (other than this one).
pub fn kick_idle() {
    let idle = IDLE.load(Ordering::SeqCst) & !(1u64 << cpu());
    if idle != 0 {
        let target = idle.trailing_zeros();
        gic::send_sgi(1 << target, SGI_WAKE);
    }
}

/// Bring up the other CPUs described by the board (PSCI CPU_ON).
pub fn start_secondaries() {
    extern "C" {
        static __secondary_entry: u8;
    }
    // SAFETY: only the symbol's address is used.
    let entry = mmu::virt_to_phys(unsafe { &__secondary_entry as *const u8 as u64 });
    let n = (platform::CPUS as usize).min(MAX_CPUS);
    for id in 1..n {
        let mpidr = platform::CPU_MPIDR.get(id).copied().unwrap_or(id as u64);
        let r = psci::cpu_on(mpidr, entry, id as u64);
        if r != 0 {
            kprintln!("smp: cpu {} did not start (PSCI {})", id, r);
        }
    }
}

/// CPU 0's identity, before anything else runs.
pub fn init_boot_cpu() {
    set_cpu(0);
}

/// First Rust code on a secondary CPU (MMU on, running in the upper half
/// on its own stack).
#[no_mangle]
extern "C" fn secondary_main(id: usize) -> ! {
    set_cpu(id);
    // SAFETY: switches this CPU to the kernel's final tables.
    unsafe { mmu::init_secondary() };
    arch::exceptions::init();
    arch::enable_fp();
    gic::init_cpu();
    arch::timer::init_cpu();
    lock();
    ONLINE.fetch_add(1, Ordering::SeqCst);
    kprintln!("smp: cpu {} online", id);
    crate::kernel::secondary_start(id)
}

/// Top of a secondary CPU's kernel stack.
pub fn stack_top(id: usize) -> u64 {
    // SAFETY: address arithmetic only.
    let base = core::ptr::addr_of!(SECONDARY_STACKS) as u64;
    base + ((id + 1) * STACK_SIZE) as u64
}
