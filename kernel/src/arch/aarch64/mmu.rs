//! Stage-1 translation for EL1/EL0.
//!
//! 4 KiB granule, 39-bit virtual addresses in both halves.
//!
//! * **TTBR1 (upper half)** belongs to the kernel: a direct map of RAM and
//!   device windows at `KVA + physical`, global and inaccessible to EL0.
//!   The kernel image runs inside it. Only what the board describes is
//!   mapped, in 2 MiB blocks where alignment allows and 4 KiB pages
//!   elsewhere: firmware carve-outs between RAM regions (TrustZone, modem
//!   and DSP memory on phone SoCs) are never mapped, not even for
//!   speculative accesses.
//! * **TTBR0 (lower half)** belongs entirely to the running component or
//!   process, with 4 KiB pages tagged by its ASID. Linux binaries can use
//!   their conventional low addresses.

use crate::global::Global;
use crate::platform;

/// Base of the kernel's direct map of physical memory.
pub const KVA: u64 = 0xffff_ff80_0000_0000;

pub const ATTR_DEVICE: u64 = 0;
pub const ATTR_NORMAL: u64 = 1;
/// Normal, non-cacheable (framebuffers): MAIR attr2 = 0x44.
pub const ATTR_NORMAL_NC: u64 = 2;

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
const MIB2: u64 = 2 << 20;
const PAGE: u64 = 4096;

/// Second- and third-level tables of the direct map.
const POOL: usize = 160;
static TABLES: Global<[Table; POOL]> = Global::new([const { Table([0; 512]) }; POOL]);
static TABLES_USED: Global<usize> = Global::new(0);

fn new_table() -> u64 {
    // SAFETY: boot-time, single-threaded.
    let used = unsafe { TABLES_USED.get() };
    assert!(*used < POOL, "mmu: direct map needs more than {} tables", POOL);
    // SAFETY: as above; the pool is zeroed .bss.
    let t = unsafe { &mut TABLES.get()[*used] };
    *used += 1;
    virt_to_phys(t.0.as_ptr() as u64)
}

fn table_at(desc: u64) -> &'static mut [u64; 512] {
    // SAFETY: `desc` is a table descriptor this module created, pointing
    // into the static pool.
    unsafe { &mut *(phys_to_virt(desc & ADDR_MASK) as *mut [u64; 512]) }
}

/// The next-level table under `*e`, creating it (an entry must not
/// already be a block: described regions do not overlap).
fn next_level(e: &mut u64) -> &'static mut [u64; 512] {
    if *e == 0 {
        *e = new_table() | VALID | TABLE;
    }
    assert!(*e & TABLE != 0, "mmu: overlapping mappings");
    table_at(*e)
}

/// Map physical `[base, base + size)` at `KVA + base` with `attrs`.
fn map(l1: &mut [u64; 512], base: u64, size: u64, attrs: u64) {
    let mut pa = base & !(PAGE - 1);
    let end = (base + size + PAGE - 1) & !(PAGE - 1);
    while pa < end {
        let l2 = next_level(&mut l1[((pa >> 30) & 0x1ff) as usize]);
        let i2 = ((pa >> 21) & 0x1ff) as usize;
        if pa % MIB2 == 0 && end - pa >= MIB2 && l2[i2] == 0 {
            l2[i2] = pa | VALID | attrs;
            pa += MIB2;
            continue;
        }
        // Already inside a block (a window that lies within another one,
        // such as a key's pin block in the GPIO controller's): it is mapped.
        if l2[i2] & VALID != 0 && l2[i2] & TABLE == 0 {
            pa = (pa & !(MIB2 - 1)) + MIB2;
            continue;
        }
        let l3 = next_level(&mut l2[i2]);
        let i3 = ((pa >> 12) & 0x1ff) as usize;
        if l3[i3] == 0 {
            l3[i3] = pa | VALID | TABLE | attrs;
        }
        pa += PAGE;
    }
}

/// Build the kernel's direct map from the board description and drop the
/// boot identity map.
///
/// # Safety
/// Must run once, at boot, before any user address space exists.
pub unsafe fn init() {
    let l1 = &mut KERNEL_L1.get().0;
    for r in platform::RAM {
        map(l1, r.base, r.size, attr(ATTR_NORMAL) | AF | SH_INNER | UXN);
    }
    for d in platform::DEVICES {
        let fb = d.is_compatible("simple-framebuffer");
        for r in d.regs {
            let attrs = if fb { attr(ATTR_NORMAL_NC) | AF | SH_INNER } else { attr(ATTR_DEVICE) | AF };
            map(l1, r.base, r.size, attrs | UXN | PXN);
        }
    }
    crate::console::early_mark(4);
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

/// Switch a secondary CPU from the boot tables to the kernel's tables.
///
/// # Safety
/// Once per secondary CPU, after the boot CPU ran `init`.
pub unsafe fn init_secondary() {
    let ttbr1 = virt_to_phys(KERNEL_L1.get().0.as_ptr() as u64);
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

/// Write back the data cache lines of `[va, va + len)` to memory, for
/// devices that read memory directly (a display engine scanning out).
pub fn clean_dcache(va: u64, len: u64) {
    let mut a = va & !63;
    while a < va + len {
        // SAFETY: cache maintenance on a mapped kernel address.
        unsafe { core::arch::asm!("dc cvac, {}", in(reg) a, options(nostack)) };
        a += 64;
    }
    // SAFETY: barrier only.
    unsafe { core::arch::asm!("dsb sy", options(nostack)) };
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
