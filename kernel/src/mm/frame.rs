//! Physical frame allocator.
//!
//! Hands out zeroed 4 KiB frames from the board's RAM (minus the kernel
//! image and reserved regions). Freed frames go on an intrusive free list
//! stored in the frames themselves (RAM is identity-mapped for the kernel).

use super::{page_ceil, page_floor, PAGE_SIZE};
use crate::arch::mmu::{phys_to_virt, virt_to_phys};
use crate::global::Global;
use crate::platform;

const MAX_RANGES: usize = 16;

struct Frames {
    /// Untouched [next, end) ranges handed out by bumping.
    ranges: [(u64, u64); MAX_RANGES],
    count: usize,
    free_list: u64,
    total: u64,
    in_use: u64,
}

static FRAMES: Global<Frames> =
    Global::new(Frames { ranges: [(0, 0); MAX_RANGES], count: 0, free_list: 0, total: 0, in_use: 0 });

fn frames() -> &'static mut Frames {
    // SAFETY: leaf module; no re-entrancy.
    unsafe { FRAMES.get() }
}

fn add_range(f: &mut Frames, start: u64, end: u64) {
    let (start, end) = (page_ceil(start), page_floor(end));
    if start < end && f.count < MAX_RANGES {
        f.ranges[f.count] = (start, end);
        f.count += 1;
        f.total += (end - start) / PAGE_SIZE;
    }
}

/// Add `[start, end)` minus every excluded range.
fn add_excluding(f: &mut Frames, start: u64, end: u64, excluded: &[(u64, u64)]) {
    if start >= end {
        return;
    }
    if let Some(&(xs, xe)) = excluded.iter().find(|&&(xs, xe)| xs < end && start < xe) {
        add_excluding(f, start, xs.max(start), excluded);
        add_excluding(f, xe.min(end), end, excluded);
    } else {
        add_range(f, start, end);
    }
}

/// `heap`: the kernel heap's arena, which the frames must not overlap.
/// `heap` and `keep` (a boot loader's ramdisk, or (0, 0)) are not RAM to
/// hand out.
pub fn init(heap: (u64, u64), keep: (u64, u64)) {
    extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    }
    // SAFETY: linker-provided symbols; only their addresses are used.
    let (ks, ke) =
        unsafe { (virt_to_phys(&__kernel_start as *const u8 as u64), virt_to_phys(&__kernel_end as *const u8 as u64)) };
    let f = frames();
    let mut excluded = [(0u64, 0u64); 35];
    excluded[0] = (0, ke); // everything below the kernel end: firmware/DTB area + image
    excluded[1] = heap;
    excluded[2] = keep;
    for (i, r) in platform::RESERVED.iter().take(32).enumerate() {
        excluded[i + 3] = (r.base, r.base + r.size);
    }
    for r in platform::RAM {
        add_excluding(f, r.base, r.base + r.size, &excluded);
    }
    kprintln!(
        "mm: kernel image {:#x}..{:#x}, {} free frames ({} MiB)",
        ks,
        ke,
        f.total,
        f.total * PAGE_SIZE / (1024 * 1024)
    );
}

/// Allocate one zeroed frame.
pub fn alloc() -> Option<u64> {
    let f = frames();
    let pa = if f.free_list != 0 {
        let pa = f.free_list;
        // SAFETY: free frames hold the next pointer in their first word.
        f.free_list = unsafe { *(phys_to_virt(pa) as *const u64) };
        pa
    } else {
        let r = f.ranges[..f.count].iter_mut().find(|(next, end)| next < end)?;
        let pa = r.0;
        r.0 += PAGE_SIZE;
        pa
    };
    f.in_use += 1;
    // SAFETY: the frame is RAM owned by nobody else, identity-mapped.
    unsafe { core::ptr::write_bytes(phys_to_virt(pa) as *mut u8, 0, PAGE_SIZE as usize) };
    Some(pa)
}

/// Allocate `pages` physically contiguous zeroed frames (for memory that
/// devices scan out). Taken from the untouched ranges; never returned.
pub fn alloc_contiguous(pages: u64) -> Option<u64> {
    let f = frames();
    let len = pages * PAGE_SIZE;
    let r = f.ranges[..f.count].iter_mut().find(|(next, end)| end - next >= len)?;
    let pa = r.0;
    r.0 += len;
    f.in_use += pages;
    // SAFETY: fresh RAM owned by nobody else, identity-mapped.
    unsafe { core::ptr::write_bytes(phys_to_virt(pa) as *mut u8, 0, len as usize) };
    Some(pa)
}

pub fn free(pa: u64) {
    let f = frames();
    // SAFETY: the caller owned this frame; it now becomes a free-list node.
    unsafe { *(phys_to_virt(pa) as *mut u64) = f.free_list };
    f.free_list = pa;
    f.in_use -= 1;
}

pub fn total() -> u64 {
    frames().total
}

pub fn in_use() -> u64 {
    frames().in_use
}
