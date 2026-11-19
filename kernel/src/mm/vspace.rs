//! Component address spaces.

use super::{frame, PAGE_SIZE};
use crate::arch::mmu::{
    self, attr, Table, ADDR_MASK, AF, AP_EL0, AP_RO, ATTR_DEVICE, ATTR_NORMAL, NG, PXN, SH_INNER, TABLE, UXN, VALID,
};
use alloc::vec::Vec;
use flare_abi::layout::{USER_REGION_BASE, USER_REGION_SIZE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapKind {
    /// R-X for EL0.
    Code,
    /// R-- for EL0.
    ReadOnly,
    /// RW- for EL0.
    Data,
    /// RW- device memory (MMIO).
    Device,
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
    // SAFETY: `pa` is a page-table frame owned by an AddressSpace; RAM is
    // identity-mapped for the kernel.
    unsafe { &mut (*(pa as *mut Table)).0 }
}

fn indices(va: u64) -> (usize, usize, usize) {
    (((va >> 30) & 511) as usize, ((va >> 21) & 511) as usize, ((va >> 12) & 511) as usize)
}

impl AddressSpace {
    pub fn new(asid: u16) -> Option<AddressSpace> {
        let l1 = frame::alloc()?;
        table(l1).copy_from_slice(mmu::kernel_entries());
        Some(AddressSpace { l1, asid, tables: Vec::new() })
    }

    pub fn asid(&self) -> u16 {
        self.asid
    }

    pub fn ttbr(&self) -> u64 {
        self.l1 | ((self.asid as u64) << 48)
    }

    fn in_user_region(va: u64) -> bool {
        (USER_REGION_BASE..USER_REGION_BASE + USER_REGION_SIZE).contains(&va)
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
        // Level-1 entries outside the user region are the kernel's blocks.
        if !Self::in_user_region(va) {
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
        let (i1, i2, i3) = indices(va);
        let e1 = table(self.l1)[i1];
        if e1 & (VALID | TABLE) != (VALID | TABLE) {
            return None;
        }
        let e2 = table(e1 & ADDR_MASK)[i2];
        if e2 & (VALID | TABLE) != (VALID | TABLE) {
            return None;
        }
        let e3 = table(e2 & ADDR_MASK)[i3];
        (e3 & VALID != 0).then_some(e3)
    }

    pub fn map(&mut self, va: u64, pa: u64, kind: MapKind) -> Result<(), MapError> {
        if !Self::in_user_region(va) || !va.is_multiple_of(PAGE_SIZE) || !pa.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::OutOfRange);
        }
        let flags = VALID
            | TABLE
            | AF
            | NG
            | PXN
            | AP_EL0
            | match kind {
                MapKind::Code => attr(ATTR_NORMAL) | SH_INNER | AP_RO,
                MapKind::ReadOnly => attr(ATTR_NORMAL) | SH_INNER | AP_RO | UXN,
                MapKind::Data => attr(ATTR_NORMAL) | SH_INNER | UXN,
                MapKind::Device => attr(ATTR_DEVICE) | UXN,
            };
        let entry = self.leaf_mut(va, true).ok_or(MapError::NoMemory)?;
        if *entry & VALID != 0 {
            return Err(MapError::AlreadyMapped);
        }
        *entry = pa | flags;
        mmu::table_barrier();
        Ok(())
    }

    pub fn unmap(&mut self, va: u64) -> Option<u64> {
        let entry = self.leaf_mut(va, false)?;
        if *entry & VALID == 0 {
            return None;
        }
        let pa = *entry & ADDR_MASK;
        *entry = 0;
        mmu::flush_page(va, self.asid);
        Some(pa)
    }

    /// Physical address behind `va`, ignoring permissions (kernel use).
    pub fn lookup(&self, va: u64) -> Option<u64> {
        self.leaf(va).map(|e| (e & ADDR_MASK) | (va & (PAGE_SIZE - 1)))
    }

    /// Physical address behind a user pointer, if EL0 may access it this way.
    /// Device memory is never accessed on a component's behalf.
    pub fn translate_user(&self, va: u64, write: bool) -> Option<u64> {
        if !Self::in_user_region(va) {
            return None;
        }
        let e = self.leaf(va)?;
        let user = e & AP_EL0 != 0;
        let writable = e & AP_RO == 0;
        let normal = (e >> 2) & 7 == ATTR_NORMAL;
        (user && normal && (!write || writable)).then_some((e & ADDR_MASK) | (va & (PAGE_SIZE - 1)))
    }

    /// Free the translation tables (mapped frames belong to their owner)
    /// and drop the ASID's TLB entries.
    pub fn destroy(self) {
        // The table may be live in TTBR0 (a component tearing itself down).
        mmu::switch_to_kernel();
        mmu::flush_asid(self.asid);
        for t in self.tables {
            frame::free(t);
        }
        frame::free(self.l1);
    }
}
