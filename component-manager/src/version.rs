//! Component versions and versioned interface references.

use alloc::format;
use alloc::string::{String, ToString};
use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    pub fn parse(s: &str) -> Result<Version, String> {
        let mut parts = s.split('.');
        let mut next = |what: &str| -> Result<u16, String> {
            parts
                .next()
                .ok_or_else(|| format!("version '{}' is missing its {} number", s, what))?
                .parse()
                .map_err(|_| format!("version '{}' has an invalid {} number", s, what))
        };
        let v = Version { major: next("major")?, minor: next("minor")?, patch: next("patch")? };
        if parts.next().is_some() {
            return Err(format!("version '{}' must be MAJOR.MINOR.PATCH", s));
        }
        Ok(v)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// `name@major.minor`, e.g. `flare.console@1.0`.
///
/// A provider of `X@M.n` satisfies a requirement for `X@M.m` when `n >= m`:
/// minor versions only add, major versions break.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InterfaceRef {
    pub name: String,
    pub major: u16,
    pub minor: u16,
}

impl InterfaceRef {
    pub fn parse(s: &str) -> Result<InterfaceRef, String> {
        let (name, ver) =
            s.split_once('@').ok_or_else(|| format!("interface '{}' must be written name@MAJOR.MINOR", s))?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') {
            return Err(format!("invalid interface name '{}'", name));
        }
        let (maj, min) =
            ver.split_once('.').ok_or_else(|| format!("interface '{}' must be written name@MAJOR.MINOR", s))?;
        let num = |x: &str| x.parse::<u16>().map_err(|_| format!("invalid version in interface '{}'", s));
        Ok(InterfaceRef { name: name.to_string(), major: num(maj)?, minor: num(min)? })
    }

    /// Does this (provided) interface satisfy `req`?
    pub fn satisfies(&self, req: &InterfaceRef) -> bool {
        self.name == req.name && self.major == req.major && self.minor >= req.minor
    }
}

impl fmt::Display for InterfaceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}.{}", self.name, self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_compatibility() {
        let p = InterfaceRef::parse("flare.console@1.2").unwrap();
        assert!(p.satisfies(&InterfaceRef::parse("flare.console@1.0").unwrap()));
        assert!(!p.satisfies(&InterfaceRef::parse("flare.console@1.3").unwrap()));
        assert!(!p.satisfies(&InterfaceRef::parse("flare.console@2.0").unwrap()));
        assert!(!p.satisfies(&InterfaceRef::parse("flare.audio@1.0").unwrap()));
        assert!(InterfaceRef::parse("flare.console").is_err());
        assert!(InterfaceRef::parse("bad name@1.0").is_err());
        assert_eq!(p.to_string(), "flare.console@1.2");
    }

    #[test]
    fn versions() {
        assert_eq!(Version::parse("0.1.2").unwrap(), Version { major: 0, minor: 1, patch: 2 });
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
    }
}
