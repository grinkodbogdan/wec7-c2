//! Stage-1 translation for EL1/EL0.
//!
//! 4 KiB granule, 39-bit virtual addresses in both halves.
//!
//! * **TTBR1 (upper half)** belongs to the kernel: a direct map of RAM and
//!   device windows at `KVA + physical`, in 1 GiB blocks, global and
//!   inaccessible to EL0. The kernel image runs inside it.
//! * **TTBR0 (lower half)** belongs entirely to the running component or
//!   process, with 4 KiB pages tagged by its ASID. Linux binaries can use
//!   their conventional low addresses.

use crate::global::Global;
use crate::platform;

/// Base of the kernel's direct map of physical memory.
pub const KVA: u64 = 0xffff_ff80_0000_0000;

pub const ATTR_DEVICE: u64 = 0;
pub const ATTR_NORMAL: u64 = 1;

pub const VALID: u64 = 1 << 0;
pub const TABLE: u64 = 1 << 1; // table (L1/L2) or page (L3) descriptor
pub const AP_EL0: u64 = 1 << 6;
pub const AP_RO: u64 = 1 << 7;
pub const SH_INNER: u64 = 3 << 8;
pub const AF: u64 = 1 << 10;
pub const NG: u64 = 1 << 11;
pub const PXN: u64 = 1 << 53;
pub const UXN: u64 = 1 << 54;
/// Software bit: the mapped frame is owned by the address space.
pub const SW_OWNED: u64 = 1 << 55;
pub const ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

pub const fn attr(idx: u64) -> u64 {
    idx << 2
}

/// Kernel virtual address of a physical address.
pub const fn phys_to_virt(pa: u64) -> u64 {
    pa + KVA
}

/// Physical address of a kernel (direct-map) virtual address.
pub const fn virt_to_phys(va: u64) -> u64 {
    va - KVA
}

#[repr(C, align(4096))]
pub struct Table(pub [u64; 512]);

static KERNEL_L1: Global<Table> = Global::new(Table([0; 512]));
static EMPTY_L1: Global<Table> = Global::new(Table([0; 512]));
const GIB: u64 = 1 << 30;

/// Build the kernel's direct map from the board description and drop the
/// boot identity map.
///
/// # Safety
/// Must run once, at boot, before any user address space exists.
pub unsafe fn init() {
    let l1 = &mut KERNEL_L1.get().0;
    for r in platform::RAM {
        for gib in (r.base / GIB)..=((r.base + r.size - 1) / GIB) {
            l1[gib as usize] = (gib * GIB) | VALID | attr(ATTR_NORMAL) | AF | SH_INNER | UXN;
        }
    }
    for d in platform::DEVICES {
        for r in d.regs {
            for gib in (r.base / GIB)..=((r.base + r.size - 1) / GIB) {
                let e = &mut l1[gib as usize];
                if *e == 0 {
                    *e = (gib * GIB) | VALID | attr(ATTR_DEVICE) | AF | UXN | PXN;
                }
            }
        }
    }
    let ttbr1 = virt_to_phys(l1.as_ptr() as u64);
    let ttbr0 = virt_to_phys(EMPTY_L1.get().0.as_ptr() as u64);
    core::arch::asm!(
        "dsb ish",
        "msr ttbr1_el1, {t1}",
        "msr ttbr0_el1, {t0}",
        "isb",
        "tlbi vmalle1",
        "dsb ish",
        "isb",
        t1 = in(reg) ttbr1,
        t0 = in(reg) ttbr0,
        options(nostack),
    );
}

/// Install a user translation table (physical base | ASID << 48).
pub fn switch_to(ttbr: u64) {
    // SAFETY: only changes the lower half; the kernel runs in the upper half.
    unsafe { core::arch::asm!("msr ttbr0_el1, {}", "isb", in(reg) ttbr, options(nostack)) };
}

/// Switch to an empty lower half (before freeing the active user tables).
pub fn switch_to_kernel() {
    // SAFETY: EMPTY_L1 is a static, never-written table.
    let t = unsafe { virt_to_phys(EMPTY_L1.get().0.as_ptr() as u64) };
    switch_to(t);
}

/// Make page-table stores visible to the table walker.
pub fn table_barrier() {
    // SAFETY: barriers only.
    unsafe { core::arch::asm!("dsb ishst", "isb", options(nostack)) };
}

pub fn flush_page(va: u64, asid: u16) {
    let arg = ((va >> 12) & 0xfff_ffff_ffff) | ((asid as u64) << 48);
    // SAFETY: TLB maintenance only.
    unsafe { core::arch::asm!("dsb ishst", "tlbi vae1is, {}", "dsb ish", "isb", in(reg) arg, options(nostack)) };
}

pub fn flush_asid(asid: u16) {
    let arg = (asid as u64) << 48;
    // SAFETY: TLB maintenance only.
    unsafe { core::arch::asm!("dsb ishst", "tlbi aside1is, {}", "dsb ish", "isb", in(reg) arg, options(nostack)) };
}
