//! The boot composition, generated at build time from `system/*.boot.toml`.
//!
//! M1 stand-in: the kernel itself starts these components. From M2 the
//! kernel starts only the user-space component manager, which then applies
//! the composition with the same `flare-cm` logic.

use flare_abi::Rights;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Restart {
    Never,
    OnFailure,
}

#[derive(Clone, Copy, Debug)]
pub enum GrantKind {
    /// Register window `reg` of `platform::DEVICES[device]`.
    Mmio {
        device: usize,
        reg: usize,
    },
    Irq {
        intid: u32,
    },
    Endpoint {
        endpoint: usize,
        rights: Rights,
        badge: u64,
    },
    Klog {
        rights: Rights,
    },
    Dma {
        size: u64,
    },
    /// `SHM_REGIONS[region]`.
    Shm {
        region: usize,
    },
    /// `INPUT_DEVICES[device]`.
    Input {
        device: usize,
    },
    /// `SERIAL_DEVICES[device]`.
    Serial {
        device: usize,
    },
}

#[derive(Debug)]
pub struct Grant {
    pub name: &'static str,
    pub kind: GrantKind,
}

pub struct BootComponent {
    pub name: &'static str,
    pub image: &'static [u8],
    pub restart: Restart,
    pub max_restarts: u32,
    pub grants: &'static [Grant],
}

include!(concat!(env!("OUT_DIR"), "/boot_gen.rs"));
