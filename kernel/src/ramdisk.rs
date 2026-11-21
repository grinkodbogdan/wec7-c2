//! A root filesystem the boot loader loaded: a phone's boot image ramdisk.
//!
//! A kernel image that a boot loader must fit below firmware memory (the
//! Xiaomi Mi A1's LK: about 23 MiB) cannot carry a full root filesystem, so
//! `cargo xtask image` puts it in the boot image's ramdisk, which the boot
//! loader copies to the board's ramdisk address (`[boot-image] base +
//! ramdisk-offset` in the FHD). It starts with a header:
//!
//! | bytes | |
//! | --- | --- |
//! | 0..8 | `FLARERD1` |
//! | 8..16 | payload length |
//! | 16..24 | checksum: the wrapping sum of the payload's little-endian u64 words (zero-padded) |
//! | 24..32 | 0 |
//!
//! then the payload: an initramfs archive in the kernel's own format. Found
//! at boot, its memory is kept from the frame allocator for good: the Linux
//! personality's files point into it.

use crate::arch::mmu::phys_to_virt;
use crate::global::Global;
use crate::{boot, platform};

const MAGIC: &[u8; 8] = b"FLARERD1";
const HEADER: u64 = 32;

static FOUND: Global<Option<(u64, u64)>> = Global::new(None);

/// Look for the ramdisk (before the frame allocator starts). Returns the
/// physical range to keep, header included.
pub fn probe() -> Option<(u64, u64)> {
    let pa = boot::RAMDISK_PHYS?;
    let in_ram = |a: u64, b: u64| platform::RAM.iter().any(|r| a >= r.base && b <= r.base + r.size);
    if !in_ram(pa, pa + HEADER) {
        kprintln!("ramdisk: {:#x} is not in RAM", pa);
        return None;
    }
    let va = phys_to_virt(pa) as *const u8;
    // SAFETY: inside a RAM region, through the direct map.
    let hdr = unsafe { core::slice::from_raw_parts(va, HEADER as usize) };
    if &hdr[..8] != MAGIC {
        kprintln!("ramdisk: none at {:#x} (the boot image has no Flare root filesystem)", pa);
        return None;
    }
    let len = u64::from_le_bytes(hdr[8..16].try_into().unwrap());
    let want = u64::from_le_bytes(hdr[16..24].try_into().unwrap());
    if !in_ram(pa, pa + HEADER + len) {
        kprintln!("ramdisk: {} bytes at {:#x} run past RAM", len, pa);
        return None;
    }
    // SAFETY: as above, the whole payload is RAM.
    let body = unsafe { core::slice::from_raw_parts(va.add(HEADER as usize), len as usize) };
    let mut sum = 0u64;
    for c in body.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        sum = sum.wrapping_add(u64::from_le_bytes(w));
    }
    if sum != want {
        kprintln!("ramdisk: checksum mismatch at {:#x} ({} bytes): damaged or stale, not used", pa, len);
        return None;
    }
    kprintln!("ramdisk: {} MiB root filesystem at {:#x}", len >> 20, pa);
    // SAFETY: boot, single-threaded.
    unsafe { *FOUND.get() = Some((pa, len)) };
    Some((pa, (pa + HEADER + len + 0xfff) & !0xfff))
}

/// The archive, if the boot loader loaded one.
pub fn archive() -> Option<&'static [u8]> {
    // SAFETY: written once at boot.
    let (pa, len) = unsafe { *FOUND.get() }?;
    // SAFETY: RAM kept from the allocator for good (probe).
    Some(unsafe { core::slice::from_raw_parts(phys_to_virt(pa + HEADER) as *const u8, len as usize) })
}
