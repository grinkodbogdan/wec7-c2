//! Loader for static ELF64 AArch64 executables: native components and
//! Linux programs.

use crate::arch::mmu::phys_to_virt;
use crate::mm::vspace::{AddressSpace, Prot};
use crate::mm::{page_ceil, page_floor, PAGE_SIZE};
use alloc::collections::BTreeMap;
use core::ops::Range;

const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const EM_AARCH64: u16 = 183;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;

/// Where things ended up, for building a Linux process's auxiliary vector.
#[derive(Clone, Debug)]
pub struct ElfInfo {
    pub entry: u64,
    pub phdr: u64,
    pub phent: u64,
    pub phnum: u64,
    /// First page above the highest segment (initial program break).
    pub end: u64,
    /// The program interpreter (dynamic linker) named by PT_INTERP.
    pub interp: Option<alloc::string::String>,
    /// Loaded segments: (start page, end page, protection).
    pub segments: alloc::vec::Vec<(u64, u64, Prot)>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Is this an ELF image this loader understands?
pub fn is_elf(image: &[u8]) -> bool {
    image.get(0..4) == Some(b"\x7fELF")
}

/// Load `image` into `vs`. Segments must fall inside `window`. Position-
/// independent executables (ET_DYN) are loaded at `pie_base`. On error the
/// caller destroys `vs`, which frees everything mapped so far.
pub fn load(image: &[u8], vs: &mut AddressSpace, window: Range<u64>, pie_base: u64) -> Result<ElfInfo, &'static str> {
    if !is_elf(image) || image.get(4) != Some(&2) || image.get(5) != Some(&1) {
        return Err("not a little-endian ELF64 image");
    }
    if u16_at(image, 18) != Some(EM_AARCH64) {
        return Err("not an AArch64 executable");
    }
    let bias = match u16_at(image, 16) {
        Some(ET_EXEC) => 0,
        Some(ET_DYN) => pie_base,
        _ => return Err("not an executable"),
    };
    let entry = u64_at(image, 24).ok_or("truncated header")? + bias;
    let phoff = u64_at(image, 32).ok_or("truncated header")? as usize;
    let phent = u16_at(image, 54).ok_or("truncated header")? as usize;
    let phnum = u16_at(image, 56).ok_or("truncated header")? as usize;
    let mut entry_ok = false;
    let mut phdr = 0;
    let mut end = 0;
    let mut segments = alloc::vec::Vec::new();
    let mut interp = None;
    // Final protection per page; pages shared by two segments get the union.
    let mut pages: BTreeMap<u64, Prot> = BTreeMap::new();

    for i in 0..phnum {
        let ph = phoff + i * phent;
        let ty = u32_at(image, ph).ok_or("truncated program header")?;
        let offset = u64_at(image, ph + 8).ok_or("truncated program header")? as usize;
        let vaddr = u64_at(image, ph + 16).ok_or("truncated program header")?.wrapping_add(bias);
        match ty {
            PT_PHDR => phdr = vaddr,
            PT_INTERP => {
                let filesz = u64_at(image, ph + 32).ok_or("truncated program header")? as usize;
                let raw = image.get(offset..offset + filesz).ok_or("bad PT_INTERP")?;
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                interp = Some(alloc::string::String::from_utf8_lossy(&raw[..end]).into_owned());
                continue;
            }
            PT_LOAD => {}
            _ => continue,
        }
        if ty != PT_LOAD {
            continue;
        }
        let flags = u32_at(image, ph + 4).ok_or("truncated program header")?;
        let filesz = u64_at(image, ph + 32).ok_or("truncated program header")?;
        let memsz = u64_at(image, ph + 40).ok_or("truncated program header")?;
        if memsz == 0 {
            continue;
        }
        let seg_end = vaddr.checked_add(memsz).ok_or("segment overflows")?;
        if vaddr < window.start || seg_end > window.end {
            return Err("segment outside the allowed address window");
        }
        if filesz > memsz || offset.checked_add(filesz as usize).is_none_or(|e| e > image.len()) {
            return Err("segment file range is invalid");
        }
        if flags & PF_W != 0 && flags & PF_X != 0 {
            return Err("writable and executable segment (W^X)");
        }
        if phdr == 0 && (offset..offset + filesz as usize).contains(&phoff) {
            phdr = vaddr + (phoff - offset) as u64;
        }
        let prot = Prot { none: false, write: flags & PF_W != 0, exec: flags & PF_X != 0 };
        let mut page = page_floor(vaddr);
        while page < page_ceil(seg_end) {
            match pages.get_mut(&page) {
                Some(p) => {
                    p.write |= prot.write;
                    p.exec |= prot.exec;
                }
                None => {
                    vs.map_new(page, Prot::RW).map_err(|_| "out of memory")?;
                    pages.insert(page, prot);
                }
            }
            page += PAGE_SIZE;
        }
        // Copy file contents page by page (new frames are zeroed).
        let mut copied = 0u64;
        while copied < filesz {
            let va = vaddr + copied;
            let pa = vs.lookup(va).ok_or("internal: segment not mapped")?;
            let n = (PAGE_SIZE - (va % PAGE_SIZE)).min(filesz - copied);
            let src = &image[offset + copied as usize..offset + (copied + n) as usize];
            // SAFETY: `pa` is a frame owned by `vs`, reached via the direct map.
            unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), phys_to_virt(pa) as *mut u8, n as usize) };
            if prot.exec {
                crate::arch::sync_icache(phys_to_virt(pa), n);
            }
            copied += n;
        }
        if prot.exec && (vaddr..seg_end).contains(&entry) {
            entry_ok = true;
        }
        end = end.max(page_ceil(seg_end));
        segments.push((page_floor(vaddr), page_ceil(seg_end), prot));
    }
    for (&page, &prot) in &pages {
        vs.protect(page, prot);
    }
    if !entry_ok {
        return Err("entry point is not inside an executable segment");
    }
    Ok(ElfInfo { entry, phdr, phent: phent as u64, phnum: phnum as u64, end, interp, segments })
}
