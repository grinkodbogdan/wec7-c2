//! Stage-1 translation for EL1/EL0.
//!
//! 4 KiB granule, 39-bit virtual addresses (translation starts at level 1,
//! 1 GiB per entry). The kernel identity-maps RAM and device windows with
//! 1 GiB blocks that only EL1 can access; every component address space
//! copies those entries and adds its own user region (see
//! `flare_abi::layout`), which uses 4 KiB pages tagged with the address
//! space's ASID.

use crate::global::Global;
use crate::platform;

pub const ATTR_DEVICE: u64 = 0;
pub const ATTR_NORMAL: u64 = 1;
/// Attr0: Device-nGnRnE (0x00); Attr1: Normal, write-back (0xff).
const MAIR: u64 = 0xff << 8;

pub const VALID: u64 = 1 << 0;
pub const TABLE: u64 = 1 << 1; // table (L1/L2) or page (L3) descriptor
pub const AP_EL0: u64 = 1 << 6;
pub const AP_RO: u64 = 1 << 7;
pub const SH_INNER: u64 = 3 << 8;
pub const AF: u64 = 1 << 10;
pub const NG: u64 = 1 << 11;
pub const PXN: u64 = 1 << 53;
pub const UXN: u64 = 1 << 54;
pub const ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

pub const fn attr(idx: u64) -> u64 {
    idx << 2
}

const T0SZ: u64 = 25; // 39-bit VA
const TCR: u64 = T0SZ
    | (1 << 8)   // IRGN0: write-back
    | (1 << 10)  // ORGN0: write-back
    | (3 << 12)  // SH0: inner shareable
    | (1 << 23)  // EPD1: no TTBR1 walks
    | (1 << 32); // IPS: 36-bit physical addresses

#[repr(C, align(4096))]
pub struct Table(pub [u64; 512]);

static KERNEL_L1: Global<Table> = Global::new(Table([0; 512]));
const GIB: u64 = 1 << 30;

/// The kernel's level-1 entries, shared by every address space.
pub fn kernel_entries() -> &'static [u64; 512] {
    // SAFETY: written once in `init`, read-only afterwards.
    unsafe { &KERNEL_L1.get().0 }
}

/// Build the kernel identity map from the board description and turn the
/// MMU on.
///
/// # Safety
/// Must run once, at boot, before anything relies on caches or atomics.
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
    let ttbr = l1.as_ptr() as u64;
    core::arch::asm!(
        "msr mair_el1, {mair}",
        "msr tcr_el1, {tcr}",
        "msr ttbr0_el1, {ttbr}",
        "dsb ish",
        "tlbi vmalle1",
        "dsb ish",
        "isb",
        "mrs {tmp}, sctlr_el1",
        "orr {tmp}, {tmp}, #(1 << 0)",   // M: MMU on
        "orr {tmp}, {tmp}, #(1 << 2)",   // C: data cache
        "orr {tmp}, {tmp}, #(1 << 12)",  // I: instruction cache
        "bic {tmp}, {tmp}, #(1 << 1)",   // A: no alignment checks
        "bic {tmp}, {tmp}, #(1 << 19)",  // WXN off (user pages are W^X by construction)
        "msr sctlr_el1, {tmp}",
        "isb",
        mair = in(reg) MAIR,
        tcr = in(reg) TCR,
        ttbr = in(reg) ttbr,
        tmp = out(reg) _,
        options(nostack),
    );
}

/// Install a component's translation table (base | ASID << 48).
pub fn switch_to(ttbr: u64) {
    // SAFETY: `ttbr` comes from a live AddressSpace, which always contains
    // the kernel mappings.
    unsafe { core::arch::asm!("msr ttbr0_el1, {}", "isb", in(reg) ttbr, options(nostack)) };
}

/// Switch to the kernel-only tables (before freeing a component's tables).
pub fn switch_to_kernel() {
    switch_to(kernel_entries().as_ptr() as u64);
}

/// Make page-table stores visible to the table walker.
pub fn table_barrier() {
    // SAFETY: barriers only.
    unsafe { core::arch::asm!("dsb ishst", "isb", options(nostack)) };
}

pub fn flush_page(va: u64, asid: u16) {
    let arg = (va >> 12) | ((asid as u64) << 48);
    // SAFETY: TLB maintenance only.
    unsafe { core::arch::asm!("dsb ishst", "tlbi vae1is, {}", "dsb ish", "isb", in(reg) arg, options(nostack)) };
}

pub fn flush_asid(asid: u16) {
    let arg = (asid as u64) << 48;
    // SAFETY: TLB maintenance only.
    unsafe { core::arch::asm!("dsb ishst", "tlbi aside1is, {}", "dsb ish", "isb", in(reg) arg, options(nostack)) };
}
