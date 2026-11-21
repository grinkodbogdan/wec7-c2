//! Root filesystem archives from outside the kernel image.
//!
//! A kernel image that a boot loader must fit below firmware memory (the
//! Xiaomi Mi A1's LK: about 23 MiB) cannot carry a full root filesystem.
//! It comes instead:
//! * from the boot image's ramdisk, which the boot loader copies to the
//!   board's ramdisk address (`[boot-image] base + ramdisk-offset`), found
//!   at boot (`[linux] rootfs = "ramdisk"`);
//! * or from storage, read by the Linux driver host (a phone's `userdata`
//!   partition through Linux's eMMC driver) into DMA memory and handed over
//!   with `FLAREHW_ROOTFS` (`[linux] rootfs = "userdata"`).
//!
//! Either way it starts with a 32-byte header:
//!
//! | bytes | `FLARERD1` (plain) | `FLARERDZ` (zlib) |
//! | --- | --- | --- |
//! | 8..16 | payload length | compressed length |
//! | 16..24 | checksum: the wrapping sum of the payload's little-endian u64 words (zero-padded) | (the same, of the compressed bytes) |
//! | 24..32 | 0 | uncompressed length |
//!
//! then the payload: an initramfs archive in the kernel's own format
//! (kernel/build.rs `pack_dir`), plain or zlib-compressed. The archive's
//! memory is kept for good: the Linux personality's files point into it.

use crate::arch::mmu::phys_to_virt;
use crate::global::Global;
use crate::mm::{frame, PAGE_SIZE};
use crate::{boot, platform};
use alloc::boxed::Box;

pub const HEADER: u64 = 32;

/// A parsed, checked header.
#[derive(Clone, Copy)]
pub struct Archive {
    /// Bytes after the header.
    pub len: u64,
    /// Uncompressed size, if the payload is zlib-compressed.
    pub zlib: Option<u64>,
}

/// Parse and check an archive at `va` (`avail` bytes there, header
/// included). The error says why it is not one.
pub fn check(va: *const u8, avail: u64) -> Result<Archive, &'static str> {
    if avail < HEADER {
        return Err("too small");
    }
    // SAFETY: the caller vouches for `avail` readable bytes at `va`.
    let hdr = unsafe { core::slice::from_raw_parts(va, HEADER as usize) };
    let zlib = match &hdr[..8] {
        b"FLARERD1" => false,
        b"FLARERDZ" => true,
        _ => return Err("no Flare root filesystem header"),
    };
    let word = |i: usize| u64::from_le_bytes(hdr[i..i + 8].try_into().unwrap());
    let (len, want, ulen) = (word(8), word(16), word(24));
    if len > avail - HEADER {
        return Err("truncated");
    }
    // SAFETY: inside the `avail` bytes.
    let body = unsafe { core::slice::from_raw_parts(va.add(HEADER as usize), len as usize) };
    let mut sum = 0u64;
    for c in body.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        sum = sum.wrapping_add(u64::from_le_bytes(w));
    }
    if sum != want {
        return Err("checksum mismatch (damaged or stale)");
    }
    Ok(Archive { len, zlib: zlib.then_some(ulen) })
}

/// The archive's contents in memory kept for good: inflated into fresh
/// frames if compressed, else copied there unless `in_place` (memory that
/// is already the kernel's for good).
pub fn materialize(va: *const u8, a: Archive, in_place: bool) -> Result<&'static [u8], &'static str> {
    // SAFETY: checked by `check`.
    let body = unsafe { core::slice::from_raw_parts(va.add(HEADER as usize), a.len as usize) };
    if a.zlib.is_none() && in_place {
        return Ok(body);
    }
    let out_len = a.zlib.unwrap_or(a.len);
    let pages = out_len.div_ceil(PAGE_SIZE).max(1);
    let pa = frame::alloc_contiguous(pages).ok_or("no contiguous memory for it")?;
    // SAFETY: fresh RAM of `pages` pages, kept for good from here on.
    let out = unsafe { core::slice::from_raw_parts_mut(phys_to_virt(pa) as *mut u8, out_len as usize) };
    match a.zlib {
        None => out.copy_from_slice(body),
        Some(ulen) => {
            use miniz_oxide::inflate::core::{decompress, inflate_flags as f, DecompressorOxide};
            use miniz_oxide::inflate::TINFLStatus;
            let mut r = Box::new(DecompressorOxide::new());
            let flags = f::TINFL_FLAG_PARSE_ZLIB_HEADER | f::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
            let (status, _, n) = decompress(&mut r, body, out, 0, flags);
            if status != TINFLStatus::Done || n as u64 != ulen {
                return Err("does not inflate");
            }
        }
    }
    Ok(out)
}

static FOUND: Global<Option<(u64, Archive)>> = Global::new(None);

/// Look for the boot loader's ramdisk (before the frame allocator starts).
/// Returns the physical range to keep, header included.
pub fn probe() -> Option<(u64, u64)> {
    let pa = boot::RAMDISK_PHYS?;
    let region = platform::RAM.iter().find(|r| pa >= r.base && pa + HEADER <= r.base + r.size);
    let Some(region) = region else {
        kprintln!("ramdisk: {:#x} is not in RAM", pa);
        return None;
    };
    match check(phys_to_virt(pa) as *const u8, region.base + region.size - pa) {
        Ok(a) => {
            kprintln!(
                "ramdisk: {} MiB root filesystem at {:#x}{}",
                a.zlib.unwrap_or(a.len) >> 20,
                pa,
                if a.zlib.is_some() { " (compressed)" } else { "" }
            );
            // SAFETY: boot, single-threaded.
            unsafe { *FOUND.get() = Some((pa, a)) };
            Some((pa, (pa + HEADER + a.len + 0xfff) & !0xfff))
        }
        Err(e) => {
            kprintln!("ramdisk: none at {:#x}: {}", pa, e);
            None
        }
    }
}

/// The boot loader's ramdisk archive, inflated if need be.
pub fn archive() -> Option<&'static [u8]> {
    // SAFETY: written once at boot.
    let (pa, a) = unsafe { *FOUND.get() }?;
    // The ramdisk's own memory is kept for good (probe).
    match materialize(phys_to_virt(pa) as *const u8, a, true) {
        Ok(s) => Some(s),
        Err(e) => {
            kprintln!("ramdisk: {}", e);
            None
        }
    }
}
