//! Driver binding: Discover/Resolve steps of the lifecycle. Described
//! devices are matched to driver components by compatible string.

use crate::manifest::{Kind, Manifest};
use alloc::string::String;
use alloc::vec::Vec;

/// What binding needs to know about a described device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    /// Most specific first.
    pub compatible: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub device: String,
    /// Index into the manifest slice, or `None` if no driver matches.
    pub driver: Option<usize>,
    /// The compatible string that matched.
    pub matched: Option<String>,
}

/// For every device, choose the driver matching its *most specific*
/// compatible string. Among drivers matching the same string, the first in
/// manifest order wins (compositions control that order explicitly).
pub fn bind(devices: &[DeviceInfo], manifests: &[Manifest]) -> Vec<Binding> {
    devices
        .iter()
        .map(|d| {
            for compat in &d.compatible {
                if let Some(i) = manifests.iter().position(|m| m.kind == Kind::Driver && m.binds.contains(compat)) {
                    return Binding { device: d.name.clone(), driver: Some(i), matched: Some(compat.clone()) };
                }
            }
            Binding { device: d.name.clone(), driver: None, matched: None }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SwapPolicy;
    use alloc::string::ToString;
    use alloc::vec;

    fn driver(name: &str, binds: &[&str]) -> Manifest {
        let mut m = Manifest::new(name, Kind::Driver, SwapPolicy::Restart, &[], &[]);
        m.binds = binds.iter().map(|s| s.to_string()).collect();
        m
    }

    #[test]
    fn most_specific_compatible_wins() {
        let ms = vec![driver("generic-primecell", &["arm,primecell"]), driver("uart-pl011", &["arm,pl011"])];
        let devs = vec![
            DeviceInfo { name: "uart0".into(), compatible: vec!["arm,pl011".into(), "arm,primecell".into()] },
            DeviceInfo { name: "rtc0".into(), compatible: vec!["arm,pl031".into(), "arm,primecell".into()] },
            DeviceInfo { name: "blk".into(), compatible: vec!["virtio,mmio".into()] },
        ];
        let b = bind(&devs, &ms);
        assert_eq!(b[0].driver, Some(1));
        assert_eq!(b[1].driver, Some(0));
        assert_eq!(b[1].matched.as_deref(), Some("arm,primecell"));
        assert_eq!(b[2].driver, None);
    }
}
