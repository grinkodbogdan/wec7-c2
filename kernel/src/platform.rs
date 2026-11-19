//! The board's hardware, generated at build time from its FHD files.

#[derive(Clone, Copy, Debug)]
pub struct MemRange {
    pub base: u64,
    pub size: u64,
}

#[derive(Debug)]
#[allow(dead_code)] // descriptive fields kept for diagnostics and future use
pub struct DeviceDesc {
    pub name: &'static str,
    pub class: &'static str,
    pub compatible: &'static [&'static str],
    pub regs: &'static [MemRange],
    pub irqs: &'static [u32],
    pub str_props: &'static [(&'static str, &'static str)],
    pub int_props: &'static [(&'static str, u64)],
}

impl DeviceDesc {
    pub fn is_compatible(&self, c: &str) -> bool {
        self.compatible.contains(&c)
    }
    pub fn str_prop(&self, key: &str) -> Option<&'static str> {
        self.str_props.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

#[allow(dead_code)]
mod generated {
    use super::{DeviceDesc, MemRange};
    include!(concat!(env!("OUT_DIR"), "/platform_gen.rs"));
}
pub use generated::*;

/// First enabled device of `class` matching one of `compatible`.
pub fn find(class: &str, compatible: &[&str]) -> Option<&'static DeviceDesc> {
    DEVICES.iter().find(|d| d.class == class && compatible.iter().any(|c| d.is_compatible(c)))
}

pub fn by_class(class: &str) -> Option<&'static DeviceDesc> {
    DEVICES.iter().find(|d| d.class == class)
}
