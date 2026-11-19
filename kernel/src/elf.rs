//! Loader for component images (static ELF64, AArch64).

use crate::mm::vspace::{AddressSpace, MapKind};
use crate::mm::{frame, page_ceil, page_floor, PAGE_SIZE};
use alloc::vec::Vec;
use flare_abi::layout::{USER_IMAGE_BASE, USER_IMAGE_END};

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const EM_AARCH64: u16 = 183;
const ET_EXEC: u16 = 2;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Load `image` into `vs`, recording allocated frames in `frames`.
/// Returns the entry point.
pub fn load(image: &[u8], vs: &mut AddressSpace, frames: &mut Vec<u64>) -> Result<u64, &'static str> {
    if image.get(0..4) != Some(b"\x7fELF") || image.get(4) != Some(&2) || image.get(5) != Some(&1) {
        return Err("not a little-endian ELF64 image");
    }
    if u16_at(image, 16) != Some(ET_EXEC) || u16_at(image, 18) != Some(EM_AARCH64) {
        return Err("not an AArch64 executable");
    }
    let entry = u64_at(image, 24).ok_or("truncated header")?;
    let phoff = u64_at(image, 32).ok_or("truncated header")? as usize;
    let phentsize = u16_at(image, 54).ok_or("truncated header")? as usize;
    let phnum = u16_at(image, 56).ok_or("truncated header")? as usize;
    let mut entry_ok = false;

    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if u32_at(image, ph).ok_or("truncated program header")? != PT_LOAD {
            continue;
        }
        let flags = u32_at(image, ph + 4).ok_or("truncated program header")?;
        let offset = u64_at(image, ph + 8).ok_or("truncated program header")? as usize;
        let vaddr = u64_at(image, ph + 16).ok_or("truncated program header")?;
        let filesz = u64_at(image, ph + 32).ok_or("truncated program header")?;
        let memsz = u64_at(image, ph + 40).ok_or("truncated program header")?;
        if memsz == 0 {
            continue;
        }
        let end = vaddr.checked_add(memsz).ok_or("segment overflows")?;
        if vaddr < USER_IMAGE_BASE || end > USER_IMAGE_END {
            return Err("segment outside the component image window");
        }
        if filesz > memsz || offset.checked_add(filesz as usize).is_none_or(|e| e > image.len()) {
            return Err("segment file range is invalid");
        }
        if flags & PF_W != 0 && flags & PF_X != 0 {
            return Err("writable and executable segment (W^X)");
        }
        let kind = if flags & PF_X != 0 {
            MapKind::Code
        } else if flags & PF_W != 0 {
            MapKind::Data
        } else {
            MapKind::ReadOnly
        };
        let mut page = page_floor(vaddr);
        while page < page_ceil(end) {
            let pa = frame::alloc().ok_or("out of memory")?;
            frames.push(pa);
            vs.map(page, pa, kind).map_err(|_| "overlapping segments")?;
            page += PAGE_SIZE;
        }
        // Copy file contents page by page (frames are already zeroed).
        let mut copied = 0u64;
        while copied < filesz {
            let va = vaddr + copied;
            let pa = vs.lookup(va).ok_or("internal: segment not mapped")?;
            let n = (PAGE_SIZE - (va % PAGE_SIZE)).min(filesz - copied);
            let src = &image[offset + copied as usize..offset + (copied + n) as usize];
            // SAFETY: `pa` is a frame we just allocated for this segment.
            unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), pa as *mut u8, n as usize) };
            if kind == MapKind::Code {
                crate::arch::sync_icache(pa, n);
            }
            copied += n;
        }
        if kind == MapKind::Code && (vaddr..end).contains(&entry) {
            entry_ok = true;
        }
    }
    if !entry_ok {
        return Err("entry point is not inside an executable segment");
    }
    Ok(entry)
}
