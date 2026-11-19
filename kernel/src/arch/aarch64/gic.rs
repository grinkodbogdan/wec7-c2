//! GICv2 interrupt controller, located through the board description.

use crate::global::Global;
use crate::platform;
use core::ptr::{read_volatile, write_volatile};

const GICD_CTLR: usize = 0x000;
const GICD_TYPER: usize = 0x004;
const GICD_ISENABLER: usize = 0x100;
const GICD_ICENABLER: usize = 0x180;
const GICD_ICPENDR: usize = 0x280;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;
const GICD_SGIR: usize = 0xf00;
const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_BPR: usize = 0x008;
const GICC_IAR: usize = 0x00c;
const GICC_EOIR: usize = 0x010;

struct Gic {
    dist: usize,
    cpu: usize,
}

static GIC: Global<Gic> = Global::new(Gic { dist: 0, cpu: 0 });

fn gic() -> &'static Gic {
    // SAFETY: written once in init.
    unsafe { GIC.get() }
}

fn w32(addr: usize, v: u32) {
    // SAFETY: GIC registers, identity-mapped device memory.
    unsafe { write_volatile(addr as *mut u32, v) }
}
fn r32(addr: usize) -> u32 {
    // SAFETY: as above.
    unsafe { read_volatile(addr as *const u32) }
}
fn w8(addr: usize, v: u8) {
    // SAFETY: as above (byte-accessible registers).
    unsafe { write_volatile(addr as *mut u8, v) }
}

pub fn init() {
    let dev =
        platform::find("intc", &["arm,gic-v2", "arm,cortex-a15-gic"]).expect("no GICv2 in the hardware description");
    // SAFETY: boot-time, single reference.
    let g = unsafe { GIC.get() };
    g.dist = super::mmu::phys_to_virt(dev.regs[0].base) as usize;
    g.cpu = super::mmu::phys_to_virt(dev.regs[1].base) as usize;
    let lines = ((r32(g.dist + GICD_TYPER) & 0x1f) as usize + 1) * 32;
    w32(g.dist + GICD_CTLR, 0);
    for i in (0..lines).step_by(32) {
        w32(g.dist + GICD_ICENABLER + i / 8, !0);
        w32(g.dist + GICD_ICPENDR + i / 8, !0);
    }
    for i in 32..lines {
        w8(g.dist + GICD_IPRIORITYR + i, 0xa0);
        w8(g.dist + GICD_ITARGETSR + i, 1);
    }
    w32(g.dist + GICD_CTLR, 1);
    init_cpu();
    kprintln!("gic: GICv2 at {:#x}/{:#x}, {} interrupt lines", dev.regs[0].base, dev.regs[1].base, lines);
}

/// The banked, per-CPU half: SGI/PPI priorities and the CPU interface.
pub fn init_cpu() {
    let g = gic();
    for i in 0..32 {
        w8(g.dist + GICD_IPRIORITYR + i, 0xa0);
    }
    w32(g.cpu + GICC_PMR, 0xf0);
    w32(g.cpu + GICC_BPR, 0);
    w32(g.cpu + GICC_CTLR, 1);
}

/// Send software-generated interrupt `id` to the CPUs in `targets`.
pub fn send_sgi(targets: u32, id: u32) {
    // SAFETY: make prior memory writes visible before the interrupt.
    unsafe { core::arch::asm!("dsb ish", options(nostack)) };
    w32(gic().dist + GICD_SGIR, ((targets & 0xff) << 16) | (id & 0xf));
}

pub fn enable(intid: u32) {
    w32(gic().dist + GICD_ISENABLER + (intid as usize / 32) * 4, 1 << (intid % 32));
}

pub fn disable(intid: u32) {
    w32(gic().dist + GICD_ICENABLER + (intid as usize / 32) * 4, 1 << (intid % 32));
}

/// Acknowledge the highest-priority pending interrupt. Returns the raw IAR
/// value (pass it to `eoi`), or `None` if nothing is pending.
pub fn ack() -> Option<u32> {
    let iar = r32(gic().cpu + GICC_IAR);
    if iar & 0x3ff >= 1020 {
        None
    } else {
        Some(iar)
    }
}

pub fn eoi(iar: u32) {
    w32(gic().cpu + GICC_EOIR, iar);
}
