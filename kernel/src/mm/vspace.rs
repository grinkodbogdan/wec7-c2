//! User address spaces (the lower half, TTBR0).
//!
//! Frames mapped with `owned = true` belong to the address space and are
//! freed when unmapped or when the address space is destroyed; device
//! windows are mapped unowned.

use super::{frame, PAGE_SIZE};
use crate::arch::mmu::{
    self, attr, phys_to_virt, Table, ADDR_MASK, AF, AP_EL0, AP_RO, ATTR_DEVICE, ATTR_NORMAL, NG, PXN, SH_INNER,
    SW_OWNED, TABLE, UXN, VALID,
};
use alloc::vec::Vec;

/// Lowest and highest+1 user addresses (39-bit lower half; page 0 unmapped).
pub const USER_MIN: u64 = PAGE_SIZE;
pub const USER_END: u64 = 1 << 39;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prot {
    /// No access at all from EL0 (PROT_NONE).
    pub none: bool,
    pub write: bool,
    pub exec: bool,
}

impl Prot {
    pub const R: Prot = Prot { none: false, write: false, exec: false };
    pub const RW: Prot = Prot { none: false, write: true, exec: false };
}

#[derive(Debug, PartialEq, Eq)]
pub enum MapError {
    OutOfRange,
    AlreadyMapped,
    NoMemory,
}

pub struct AddressSpace {
    l1: u64,
    asid: u16,
    /// Level-2/3 table frames owned by this address space.
    tables: Vec<u64>,
}

fn table(pa: u64) -> &'static mut [u64; 512] {
    // SAFETY: `pa` is a page-table frame owned by an AddressSpace, reached
    // through the kernel's direct map.
    unsafe { &mut (*(phys_to_virt(pa) as *mut Table)).0 }
}

fn indices(va: u64) -> (usize, usize, usize) {
    (((va >> 30) & 511) as usize, ((va >> 21) & 511) as usize, ((va >> 12) & 511) as usize)
}

fn flags(prot: Prot, device: bool) -> u64 {
    let mut f = VALID | TABLE | AF | NG | PXN | if prot.none { 0 } else { AP_EL0 };
    if device {
        f |= attr(ATTR_DEVICE) | UXN;
    } else {
        f |= attr(ATTR_NORMAL) | SH_INNER;
    }
    if !prot.write {
        f |= AP_RO;
    }
    if !prot.exec {
        f |= UXN;
    }
    f
}

impl AddressSpace {
    pub fn new(asid: u16) -> Option<AddressSpace> {
        let l1 = frame::alloc()?;
        Some(AddressSpace { l1, asid, tables: Vec::new() })
    }

    pub fn asid(&self) -> u16 {
        self.asid
    }

    pub fn ttbr(&self) -> u64 {
        self.l1 | ((self.asid as u64) << 48)
    }

    pub fn in_user_range(va: u64) -> bool {
        (USER_MIN..USER_END).contains(&va)
    }

    fn next_level(&mut self, entry: &mut u64) -> Option<u64> {
        if *entry & VALID == 0 {
            let t = frame::alloc()?;
            self.tables.push(t);
            *entry = t | VALID | TABLE;
        }
        Some(*entry & ADDR_MASK)
    }

    fn leaf_mut(&mut self, va: u64, create: bool) -> Option<&'static mut u64> {
        if !Self::in_user_range(va) {
            return None;
        }
        let (i1, i2, i3) = indices(va);
        let l1 = table(self.l1);
        if !create && l1[i1] & VALID == 0 {
            return None;
        }
        let l2 = self.next_level(&mut l1[i1])?;
        let l2t = table(l2);
        if !create && l2t[i2] & VALID == 0 {
            return None;
        }
        let l3 = self.next_level(&mut l2t[i2])?;
        Some(&mut table(l3)[i3])
    }

    fn leaf(&self, va: u64) -> Option<u64> {
        if !Self::in_user_range(va) {
            return None;
        }
        let (i1, i2, i3) = indices(va);
        let e1 = table(self.l1)[i1];
        if e1 & VALID == 0 {
            return None;
        }
        let e2 = table(e1 & ADDR_MASK)[i2];
        if e2 & VALID == 0 {
            return None;
        }
        let e3 = table(e2 & ADDR_MASK)[i3];
        (e3 & VALID != 0).then_some(e3)
    }

    /// Map one page.
    pub fn map_page(&mut self, va: u64, pa: u64, prot: Prot, device: bool, owned: bool) -> Result<(), MapError> {
        if !Self::in_user_range(va) || !va.is_multiple_of(PAGE_SIZE) || !pa.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::OutOfRange);
        }
        let entry = self.leaf_mut(va, true).ok_or(MapError::NoMemory)?;
        if *entry & VALID != 0 {
            return Err(MapError::AlreadyMapped);
        }
        *entry = pa | flags(prot, device) | if owned { SW_OWNED } else { 0 };
        mmu::table_barrier();
        Ok(())
    }

    /// Map one page of physical memory the caller vouches for (a granted
    /// device window or DMA memory) with memory attribute `attr`
    /// (mmu::ATTR_*), not owned.
    pub fn map_phys(&mut self, va: u64, pa: u64, prot: Prot, attr_idx: u64) -> Result<(), MapError> {
        if !Self::in_user_range(va) || !va.is_multiple_of(PAGE_SIZE) || !pa.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::OutOfRange);
        }
        let entry = self.leaf_mut(va, true).ok_or(MapError::NoMemory)?;
        if *entry & VALID != 0 {
            return Err(MapError::AlreadyMapped);
        }
        let mut f = flags(prot, attr_idx == ATTR_DEVICE);
        if attr_idx == mmu::ATTR_NORMAL_NC {
            f = (f & !(7 << 2)) | attr(mmu::ATTR_NORMAL_NC);
        }
        *entry = pa | f;
        mmu::table_barrier();
        Ok(())
    }

    /// Allocate a zeroed frame and map it. Returns the physical address.
    pub fn map_new(&mut self, va: u64, prot: Prot) -> Result<u64, MapError> {
        let pa = frame::alloc().ok_or(MapError::NoMemory)?;
        if let Err(e) = self.map_page(va, pa, prot, false, true) {
            frame::free(pa);
            return Err(e);
        }
        Ok(pa)
    }

    /// Unmap one page, freeing it if owned. Returns whether it was mapped.
    pub fn unmap(&mut self, va: u64) -> bool {
        let Some(entry) = self.leaf_mut(va, false) else { return false };
        if *entry & VALID == 0 {
            return false;
        }
        let e = core::mem::replace(entry, 0);
        mmu::flush_page(va, self.asid);
        if e & SW_OWNED != 0 {
            frame::free(e & ADDR_MASK);
        }
        true
    }

    /// Remove a page's entry without freeing its frame (for moving it).
    pub fn take_entry(&mut self, va: u64) -> Option<u64> {
        let entry = self.leaf_mut(va, false)?;
        if *entry & VALID == 0 {
            return None;
        }
        let e = core::mem::replace(entry, 0);
        mmu::flush_page(va, self.asid);
        Some(e)
    }

    /// Install an entry taken with `take_entry` at a new address.
    pub fn put_entry(&mut self, va: u64, e: u64) -> Result<(), MapError> {
        let entry = self.leaf_mut(va, true).ok_or(MapError::NoMemory)?;
        if *entry & VALID != 0 {
            return Err(MapError::AlreadyMapped);
        }
        *entry = e;
        mmu::table_barrier();
        Ok(())
    }

    /// Change the protection of a mapped page.
    pub fn protect(&mut self, va: u64, prot: Prot) -> bool {
        let Some(entry) = self.leaf_mut(va, false) else { return false };
        if *entry & VALID == 0 {
            return false;
        }
        let device = (*entry >> 2) & 7 == ATTR_DEVICE;
        *entry = (*entry & (ADDR_MASK | SW_OWNED)) | flags(prot, device);
        mmu::flush_page(va, self.asid);
        true
    }

    pub fn is_mapped(&self, va: u64) -> bool {
        self.leaf(va).is_some()
    }

    /// Physical address behind `va`, ignoring permissions (kernel use).
    pub fn lookup(&self, va: u64) -> Option<u64> {
        self.leaf(va).map(|e| (e & ADDR_MASK) | (va & (PAGE_SIZE - 1)))
    }

    /// Physical address behind a user pointer, if EL0 may access it this way.
    /// Device memory is never accessed on a component's behalf.
    pub fn translate_user(&self, va: u64, write: bool) -> Option<u64> {
        let e = self.leaf(va)?;
        let user = e & AP_EL0 != 0;
        let writable = e & AP_RO == 0;
        let normal = (e >> 2) & 7 == ATTR_NORMAL;
        (user && normal && (!write || writable)).then_some((e & ADDR_MASK) | (va & (PAGE_SIZE - 1)))
    }

    /// Visit every mapped page: (va, entry).
    fn for_each(&self, mut f: impl FnMut(u64, u64)) {
        let l1 = table(self.l1);
        for (i1, &e1) in l1.iter().enumerate() {
            if e1 & VALID == 0 {
                continue;
            }
            let l2 = table(e1 & ADDR_MASK);
            for (i2, &e2) in l2.iter().enumerate() {
                if e2 & VALID == 0 {
                    continue;
                }
                let l3 = table(e2 & ADDR_MASK);
                for (i3, &e3) in l3.iter().enumerate() {
                    if e3 & VALID != 0 {
                        f(((i1 as u64) << 30) | ((i2 as u64) << 21) | ((i3 as u64) << 12), e3);
                    }
                }
            }
        }
    }

    /// Copy every owned page into a fresh address space (fork).
    pub fn duplicate(&self, asid: u16) -> Option<AddressSpace> {
        let mut child = AddressSpace::new(asid)?;
        let mut ok = true;
        self.for_each(|va, e| {
            if !ok {
                return;
            }
            if e & SW_OWNED == 0 {
                ok = child.leaf_mut(va, true).map(|ce| *ce = e).is_some();
                return;
            }
            let Some(npa) = frame::alloc() else {
                ok = false;
                return;
            };
            // SAFETY: both are whole RAM frames reached through the direct map.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    phys_to_virt(e & ADDR_MASK) as *const u8,
                    phys_to_virt(npa) as *mut u8,
                    PAGE_SIZE as usize,
                )
            };
            match child.leaf_mut(va, true) {
                Some(ce) => *ce = npa | (e & !ADDR_MASK),
                None => {
                    frame::free(npa);
                    ok = false;
                }
            }
        });
        mmu::table_barrier();
        if ok {
            Some(child)
        } else {
            child.destroy();
            None
        }
    }

    /// Free owned frames and translation tables, and drop the ASID's TLB
    /// entries.
    pub fn destroy(self) {
        // The table may be live in TTBR0 (a component tearing itself down).
        mmu::switch_to_kernel();
        mmu::flush_asid(self.asid);
        self.for_each(|_, e| {
            if e & SW_OWNED != 0 {
                frame::free(e & ADDR_MASK);
            }
        });
        for t in self.tables {
            frame::free(t);
        }
        frame::free(self.l1);
    }
}
