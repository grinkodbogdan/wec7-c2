//! The Flare framekernel ABI (version 0).
//!
//! This crate is the single source of truth for the contract between the
//! framekernel and user-space components: system-call numbers, error codes,
//! capability kinds and rights, the IPC message format, the boot-info page
//! handed to every component, and the component address-space layout.
//!
//! See `docs/abi.md` for the prose version of this contract.

#![no_std]

use core::fmt;
use core::ops::BitOr;

pub const ABI_VERSION: u32 = 0;

/// System-call numbers. The number goes in `x8`, arguments in `x0..x7`,
/// and the status (0 = success, otherwise an [`Error`] code) comes back in `x0`.
pub mod syscall {
    /// `(klog_slot, ptr, len)` — write bytes to the kernel log. Needs `Klog` + WRITE.
    pub const DEBUG_WRITE: u64 = 0;
    /// `()` — give up the rest of the time slice.
    pub const YIELD: u64 = 1;
    /// `(ep_slot, label, w0..w5)` — blocking send. Needs `Endpoint` + SEND.
    pub const SEND: u64 = 2;
    /// `(ep_slot)` — blocking receive. Needs `Endpoint` + RECV.
    /// Returns `x1 = badge, x2 = label, x3..x8 = words`.
    pub const RECV: u64 = 3;
    /// `(ep_slot, label, w0..w5)` — send and wait for the reply. Needs SEND.
    /// Returns like `RECV` (badge is 0).
    pub const CALL: u64 = 4;
    /// `(label, w0..w5)` — reply to the component whose CALL was last received.
    pub const REPLY: u64 = 5;
    /// `(slot, rights, badge)` — derive a child capability with a subset of the
    /// rights into a free slot (returned in `x1`). A non-zero badge may only be
    /// set on an unbadged capability.
    pub const CAP_DERIVE: u64 = 6;
    /// `(slot)` — delete every capability derived from this one, anywhere.
    pub const CAP_REVOKE: u64 = 7;
    /// `(slot)` — delete this capability; its children are re-parented.
    pub const CAP_DELETE: u64 = 8;
    /// `(irq_slot, ep_slot, bit)` — deliver the interrupt as notification `bit`
    /// on an endpoint the caller can receive from. The line is masked after each
    /// delivery until `IRQ_ACK`.
    pub const IRQ_BIND: u64 = 9;
    /// `(irq_slot)` — unmask the interrupt line after handling it.
    pub const IRQ_ACK: u64 = 10;
    /// `(klog_slot, ep_slot, bit)` — become the kernel-log reader. Needs READ.
    pub const KLOG_BIND: u64 = 11;
    /// `(klog_slot, ptr, len)` — drain kernel-log bytes; count returned in `x1`.
    pub const KLOG_READ: u64 = 12;
    /// `(code)` — terminate the calling component. Never returns.
    pub const EXIT: u64 = 13;
    /// `(klog_slot, ptr, len)` — deliver console input to the kernel's
    /// Linux TTY. Needs `Klog` + WRITE (held by the console driver).
    pub const TTY_INPUT: u64 = 14;
    /// `(dma_slot, offset)` — physical address of a byte of a DMA region
    /// (for programming a device); returned in `x1`.
    pub const DMA_PADDR: u64 = 15;
}

/// Error codes returned in `x0`.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The slot is empty (or out of range): the caller holds no such authority.
    NoCap = 1,
    /// The capability exists but refers to the wrong kind of object.
    WrongType = 2,
    /// The capability lacks a right the operation needs.
    Rights = 3,
    InvalidArg = 4,
    NoMemory = 5,
    /// No free capability slot.
    NoSlot = 6,
    /// The peer component died while we were waiting for it.
    PeerDied = 7,
    /// A user pointer was unmapped or not accessible.
    BadAddress = 8,
    /// There is no pending call to reply to.
    NoReply = 9,
    UnknownSyscall = 10,
}

impl Error {
    pub fn from_code(code: u64) -> Error {
        match code {
            1 => Error::NoCap,
            2 => Error::WrongType,
            3 => Error::Rights,
            4 => Error::InvalidArg,
            5 => Error::NoMemory,
            6 => Error::NoSlot,
            7 => Error::PeerDied,
            8 => Error::BadAddress,
            9 => Error::NoReply,
            _ => Error::UnknownSyscall,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Error::NoCap => "E_NO_CAP",
            Error::WrongType => "E_WRONG_TYPE",
            Error::Rights => "E_RIGHTS",
            Error::InvalidArg => "E_INVALID_ARG",
            Error::NoMemory => "E_NO_MEMORY",
            Error::NoSlot => "E_NO_SLOT",
            Error::PeerDied => "E_PEER_DIED",
            Error::BadAddress => "E_BAD_ADDRESS",
            Error::NoReply => "E_NO_REPLY",
            Error::UnknownSyscall => "E_UNKNOWN_SYSCALL",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Decode a status word.
pub fn status(code: u64) -> Result<(), Error> {
    if code == 0 {
        Ok(())
    } else {
        Err(Error::from_code(code))
    }
}

/// Capability rights. Derivation may only ever remove rights.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Rights(pub u32);

impl Rights {
    pub const NONE: Rights = Rights(0);
    pub const READ: Rights = Rights(1 << 0);
    pub const WRITE: Rights = Rights(1 << 1);
    pub const SEND: Rights = Rights(1 << 2);
    pub const RECV: Rights = Rights(1 << 3);
    pub const GRANT: Rights = Rights(1 << 4);
    pub const MAP: Rights = Rights(1 << 5);
    pub const ALL: Rights = Rights(0x3f);

    const NAMES: [(&'static str, Rights); 6] = [
        ("read", Rights::READ),
        ("write", Rights::WRITE),
        ("send", Rights::SEND),
        ("recv", Rights::RECV),
        ("grant", Rights::GRANT),
        ("map", Rights::MAP),
    ];

    pub const fn contains(self, other: Rights) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn is_subset_of(self, other: Rights) -> bool {
        other.contains(self)
    }
    pub const fn intersect(self, other: Rights) -> Rights {
        Rights(self.0 & other.0)
    }
    pub const fn is_valid(self) -> bool {
        self.0 & !Rights::ALL.0 == 0
    }
    /// Parse `send+recv` style right lists.
    pub fn parse(s: &str) -> Option<Rights> {
        let mut r = Rights::NONE;
        for tok in s.split('+') {
            let (_, bit) = Rights::NAMES.iter().find(|(n, _)| *n == tok)?;
            r = r | *bit;
        }
        Some(r)
    }
}

impl BitOr for Rights {
    type Output = Rights;
    fn bitor(self, rhs: Rights) -> Rights {
        Rights(self.0 | rhs.0)
    }
}

impl fmt::Debug for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("none");
        }
        let mut first = true;
        for (name, bit) in Rights::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

/// Kind of object a capability refers to.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapKind {
    Null = 0,
    /// Synchronous IPC endpoint (also carries notification bits).
    Endpoint = 1,
    /// A device MMIO register window, mapped into the holder.
    Mmio = 2,
    /// A hardware interrupt line.
    Irq = 3,
    /// The kernel log.
    Klog = 4,
    /// Memory a driver may hand to its device for DMA, mapped into it.
    Dma = 5,
}

impl CapKind {
    pub fn from_u32(v: u32) -> CapKind {
        match v {
            1 => CapKind::Endpoint,
            2 => CapKind::Mmio,
            3 => CapKind::Irq,
            4 => CapKind::Klog,
            5 => CapKind::Dma,
            _ => CapKind::Null,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            CapKind::Null => "null",
            CapKind::Endpoint => "endpoint",
            CapKind::Mmio => "mmio",
            CapKind::Irq => "irq",
            CapKind::Klog => "klog",
            CapKind::Dma => "dma",
        }
    }
}

/// Number of payload words in an IPC message (besides badge and label).
pub const MSG_WORDS: usize = 6;
/// Label reserved for kernel notifications (interrupts, log activity).
/// The notification bits arrive in `words[0]`.
pub const LABEL_NOTIFY: u64 = u64::MAX;

/// An IPC message as seen by the receiver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Message {
    /// Badge of the sender's capability (identifies the client to a server).
    pub badge: u64,
    pub label: u64,
    pub words: [u64; MSG_WORDS],
}

/// Fixed virtual-address layout of every component address space.
pub mod layout {
    /// Start of the user region (one 1 GiB translation block).
    pub const USER_REGION_BASE: u64 = 0x2_0000_0000;
    pub const USER_REGION_SIZE: u64 = 0x4000_0000;
    /// Component ELF images must be linked inside this window.
    pub const USER_IMAGE_BASE: u64 = 0x2_0000_0000;
    pub const USER_IMAGE_END: u64 = 0x2_1000_0000;
    /// MMIO capabilities are mapped here, in grant order.
    pub const USER_MMIO_BASE: u64 = 0x2_1000_0000;
    pub const USER_MMIO_END: u64 = 0x2_2000_0000;
    /// The read-only [`BootInfo`](super::BootInfo) page.
    pub const USER_BOOTINFO: u64 = 0x2_2000_0000;
    pub const USER_STACK_TOP: u64 = 0x2_4000_0000;
    pub const USER_STACK_SIZE: u64 = 0x8000;
}

pub const BOOTINFO_MAGIC: u64 = u64::from_le_bytes(*b"FLAREBI0");
pub const MAX_CAPS: usize = 32;
pub const CAP_NAME_LEN: usize = 24;
pub const COMPONENT_NAME_LEN: usize = 32;

/// Describes one capability installed in the component's capability space.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CapDesc {
    pub slot: u32,
    pub kind: u32,
    /// For MMIO capabilities: where the registers are mapped.
    pub vaddr: u64,
    pub size: u64,
    /// The local name given by the boot composition (NUL padded).
    pub name: [u8; CAP_NAME_LEN],
}

impl CapDesc {
    pub const EMPTY: CapDesc = CapDesc { slot: 0, kind: 0, vaddr: 0, size: 0, name: [0; CAP_NAME_LEN] };
    pub fn name(&self) -> &str {
        str_from_fixed(&self.name)
    }
    pub fn kind(&self) -> CapKind {
        CapKind::from_u32(self.kind)
    }
    pub fn slot(&self) -> u64 {
        self.slot as u64
    }
}

/// Page mapped read-only at [`layout::USER_BOOTINFO`]; its address is passed
/// to the component entry point in `x0`. Components find their authority by
/// name here instead of assuming slot numbers.
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub abi_version: u32,
    pub cap_count: u32,
    pub name: [u8; COMPONENT_NAME_LEN],
    /// 0 on first start, incremented on every restart.
    pub incarnation: u32,
    pub _reserved: u32,
    pub caps: [CapDesc; MAX_CAPS],
}

impl BootInfo {
    pub fn name(&self) -> &str {
        str_from_fixed(&self.name)
    }
    pub fn caps(&self) -> &[CapDesc] {
        &self.caps[..(self.cap_count as usize).min(MAX_CAPS)]
    }
    pub fn find(&self, name: &str) -> Option<&CapDesc> {
        self.caps().iter().find(|c| c.name() == name)
    }
}

pub fn str_from_fixed(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..end]).unwrap_or("?")
}

/// Copy `s` into a NUL-padded fixed buffer (truncating).
pub fn fixed_from_str<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let n = s.len().min(N.saturating_sub(1));
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    out
}

/// IPC protocols of shared system services.
pub mod proto {
    /// `flare.block@1.0` — a block device in 512-byte sectors.
    ///
    /// Requests are CALLs with the operation as the label. Data travels in
    /// message words, 48 bytes per call.
    pub mod block {
        pub const INTERFACE: &str = "flare.block@1.0";
        pub const SECTOR: usize = 512;
        /// Reply: `words[0]` = capacity in sectors, `words[1]` = the
        /// driver's incarnation (changes when it is restarted or replaced).
        pub const INFO: u64 = 1;
        /// `words[0]` = sector, `words[1]` = byte offset in it. Reply label =
        /// bytes returned (up to 48, in the words), or `u64::MAX` on error.
        pub const READ: u64 = 2;
    }

    /// `flare.rtc@1.0` — the wall clock.
    pub mod rtc {
        pub const INTERFACE: &str = "flare.rtc@1.0";
        /// Reply: `words[0]` = seconds since 1970.
        pub const GET_TIME: u64 = 1;
    }

    /// `flare.console@1.0` — byte-stream console service.
    ///
    /// Requests are CALLs. The label carries the operation in its low 32 bits
    /// and a byte count in its high 32 bits; bytes are packed little-endian
    /// into the message words.
    pub mod console {
        use crate::MSG_WORDS;

        pub const INTERFACE: &str = "flare.console@1.0";
        /// Write `len` bytes. Reply label 0 on success.
        pub const WRITE: u32 = 1;
        /// Fetch buffered input. Reply label = number of bytes returned.
        pub const POLL_INPUT: u32 = 2;
        pub const MAX_CHUNK: usize = MSG_WORDS * 8;

        pub const fn label(op: u32, len: usize) -> u64 {
            (op as u64) | ((len as u64) << 32)
        }
        pub const fn split_label(label: u64) -> (u32, usize) {
            (label as u32, (label >> 32) as usize)
        }
        pub fn pack(bytes: &[u8]) -> [u64; MSG_WORDS] {
            let mut buf = [0u8; MAX_CHUNK];
            let n = bytes.len().min(MAX_CHUNK);
            buf[..n].copy_from_slice(&bytes[..n]);
            let mut words = [0u64; MSG_WORDS];
            for (i, w) in words.iter_mut().enumerate() {
                let mut b = [0u8; 8];
                b.copy_from_slice(&buf[i * 8..i * 8 + 8]);
                *w = u64::from_le_bytes(b);
            }
            words
        }
        pub fn unpack(words: &[u64; MSG_WORDS]) -> [u8; MAX_CHUNK] {
            let mut buf = [0u8; MAX_CHUNK];
            for (i, w) in words.iter().enumerate() {
                buf[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
            }
            buf
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rights_parse_and_subset() {
        let sr = Rights::parse("send+recv").unwrap();
        assert!(Rights::SEND.is_subset_of(sr));
        assert!(!sr.is_subset_of(Rights::SEND));
        assert_eq!(Rights::parse("send+bogus"), None);
        extern crate std;
        use std::string::ToString;
        assert_eq!(sr.to_string(), "send+recv");
        assert!(!Rights(0x40).is_valid());
    }

    #[test]
    fn console_packing_roundtrip() {
        let text = b"hello, flare console protocol!";
        let words = proto::console::pack(text);
        let back = proto::console::unpack(&words);
        assert_eq!(&back[..text.len()], text);
        let label = proto::console::label(proto::console::WRITE, text.len());
        assert_eq!(proto::console::split_label(label), (proto::console::WRITE, text.len()));
    }

    #[test]
    fn bootinfo_fits_in_a_page() {
        assert!(core::mem::size_of::<BootInfo>() <= 4096);
        let name: [u8; CAP_NAME_LEN] = fixed_from_str("console");
        assert_eq!(str_from_fixed(&name), "console");
    }

    #[test]
    fn error_codes_roundtrip() {
        for e in [Error::NoCap, Error::Rights, Error::NoReply, Error::PeerDied] {
            assert_eq!(Error::from_code(e as u64), e);
        }
        assert_eq!(status(0), Ok(()));
    }
}
