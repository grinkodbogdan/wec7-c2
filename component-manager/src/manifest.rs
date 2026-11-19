//! Component manifests (`component.toml`).
//!
//! ```toml
//! [component]
//! name = "uart-pl011"
//! version = "0.1.0"
//! kind = "driver"          # driver | service | app | personality
//! swap = "restart"         # live | restart | reset
//!
//! [binds]                  # drivers only: device compatible strings
//! compatible = ["arm,pl011"]
//!
//! [provides]
//! interfaces = ["flare.console@1.0"]
//!
//! [requires]
//! interfaces = []
//!
//! [lifecycle]              # optional hooks the component implements
//! quiesce = false
//! checkpoint = false
//! ```

use crate::version::{InterfaceRef, Version};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use flare_config::{Document, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Driver,
    Service,
    App,
    /// A compatibility environment (Linux ABI, Android, ...).
    Personality,
}

/// How a component can be replaced while the system runs. Ordered from
/// least to most disruptive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SwapPolicy {
    /// Replace without clients noticing: quiesce, checkpoint, transfer, restore.
    Live,
    /// Replace with a service interruption; clients reconnect.
    Restart,
    /// Replacement requires a hardware reset of the device.
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hooks {
    pub quiesce: bool,
    pub checkpoint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: Version,
    pub kind: Kind,
    pub swap: SwapPolicy,
    pub binds: Vec<String>,
    pub provides: Vec<InterfaceRef>,
    pub requires: Vec<InterfaceRef>,
    pub hooks: Hooks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub line: u32,
    pub message: String,
}

impl core::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub fn parse(src: &str) -> Result<Manifest, Vec<ManifestError>> {
    let doc = flare_config::parse(src).map_err(|e| alloc::vec![ManifestError { line: e.line, message: e.message }])?;
    from_document(&doc)
}

pub fn from_document(doc: &Document) -> Result<Manifest, Vec<ManifestError>> {
    let mut errs = Vec::new();
    let mut err = |line: u32, m: String| errs.push(ManifestError { line, message: m });

    for e in &doc.root().entries {
        err(e.line, format!("key '{}' must be inside a section", e.key));
    }
    let allowed: &[(&str, &[&str])] = &[
        ("component", &["name", "version", "kind", "swap"]),
        ("binds", &["compatible"]),
        ("provides", &["interfaces"]),
        ("requires", &["interfaces"]),
        ("lifecycle", &["quiesce", "checkpoint"]),
    ];
    for s in doc.named_sections() {
        match allowed.iter().find(|(n, _)| *n == s.name) {
            None => err(s.line, format!("unknown section [{}]", s.name)),
            Some((_, keys)) => {
                for e in &s.entries {
                    if !keys.contains(&e.key.as_str()) {
                        err(e.line, format!("unknown key '{}' in [{}]", e.key, s.name));
                    }
                }
            }
        }
    }

    let Some(c) = doc.section("component") else {
        err(1, "missing [component] section".into());
        return Err(errs);
    };
    let text = |k: &str, err: &mut dyn FnMut(u32, String)| -> Option<String> {
        match c.get(k) {
            Some(Value::Str(s)) => Some(s.clone()),
            Some(v) => {
                err(c.line_of(k), format!("component.{} must be a string, found {}", k, v.type_name()));
                None
            }
            None => {
                err(c.line, format!("component.{} is required", k));
                None
            }
        }
    };
    let name = text("name", &mut err);
    if let Some(n) = &name {
        if n.is_empty()
            || n.len() > 31
            || !n.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        {
            err(c.line_of("name"), format!("component name '{}' must be 1-31 chars of [a-z0-9-]", n));
        }
    }
    let version =
        text("version", &mut err).and_then(|v| Version::parse(&v).map_err(|m| err(c.line_of("version"), m)).ok());
    let kind = text("kind", &mut err).and_then(|k| match k.as_str() {
        "driver" => Some(Kind::Driver),
        "service" => Some(Kind::Service),
        "app" => Some(Kind::App),
        "personality" => Some(Kind::Personality),
        _ => {
            err(c.line_of("kind"), format!("unknown kind '{}' (driver, service, app, personality)", k));
            None
        }
    });
    let swap = match c.get("swap").and_then(|v| v.as_str()) {
        None if c.get("swap").is_none() => Some(SwapPolicy::Restart),
        Some("live") => Some(SwapPolicy::Live),
        Some("restart") => Some(SwapPolicy::Restart),
        Some("reset") => Some(SwapPolicy::Reset),
        _ => {
            err(c.line_of("swap"), "swap must be \"live\", \"restart\" or \"reset\"".into());
            None
        }
    };

    let list = |sec: &str, key: &str, err: &mut dyn FnMut(u32, String)| -> Vec<String> {
        let Some(s) = doc.section(sec) else { return Vec::new() };
        match s.get(key) {
            None => Vec::new(),
            Some(v) => match v.as_str_list() {
                Some(l) => l,
                None => {
                    err(s.line_of(key), format!("{}.{} must be an array of strings", sec, key));
                    Vec::new()
                }
            },
        }
    };
    let ifaces = |sec: &str, err: &mut dyn FnMut(u32, String)| -> Vec<InterfaceRef> {
        let line = doc.section(sec).map(|s| s.line_of("interfaces")).unwrap_or(0);
        list(sec, "interfaces", err)
            .iter()
            .filter_map(|s| InterfaceRef::parse(s).map_err(|m| err(line, m)).ok())
            .collect()
    };
    let binds = list("binds", "compatible", &mut err);
    let provides = ifaces("provides", &mut err);
    let requires = ifaces("requires", &mut err);
    let mut hooks = Hooks::default();
    if let Some(l) = doc.section("lifecycle") {
        for (key, slot) in [("quiesce", &mut hooks.quiesce), ("checkpoint", &mut hooks.checkpoint)] {
            match l.get(key) {
                None => {}
                Some(Value::Bool(b)) => *slot = *b,
                Some(_) => err(l.line_of(key), format!("lifecycle.{} must be true or false", key)),
            }
        }
    }
    if kind == Some(Kind::Driver) && binds.is_empty() {
        err(c.line, "drivers must declare [binds] compatible = [...]".into());
    }
    if kind.is_some() && kind != Some(Kind::Driver) && !binds.is_empty() {
        err(c.line, "only drivers bind to hardware".into());
    }
    if swap == Some(SwapPolicy::Live) && !hooks.quiesce {
        err(c.line_of("swap"), "swap = \"live\" requires the quiesce lifecycle hook".into());
    }

    match (name, version, kind, swap) {
        (Some(name), Some(version), Some(kind), Some(swap)) if errs.is_empty() => {
            Ok(Manifest { name, version, kind, swap, binds, provides, requires, hooks })
        }
        _ => Err(errs),
    }
}

impl Manifest {
    /// Convenience constructor for tests and tooling.
    pub fn new(name: &str, kind: Kind, swap: SwapPolicy, provides: &[&str], requires: &[&str]) -> Manifest {
        Manifest {
            name: name.to_string(),
            version: Version { major: 0, minor: 1, patch: 0 },
            kind,
            swap,
            binds: Vec::new(),
            provides: provides.iter().map(|s| InterfaceRef::parse(s).unwrap()).collect(),
            requires: requires.iter().map(|s| InterfaceRef::parse(s).unwrap()).collect(),
            hooks: Hooks { quiesce: swap == SwapPolicy::Live, checkpoint: false },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_driver_manifest() {
        let m = parse(
            r#"
[component]
name = "uart-pl011"
version = "0.1.0"
kind = "driver"
swap = "restart"
[binds]
compatible = ["arm,pl011"]
[provides]
interfaces = ["flare.console@1.0"]
"#,
        )
        .unwrap();
        assert_eq!(m.kind, Kind::Driver);
        assert_eq!(m.binds, ["arm,pl011"]);
        assert_eq!(m.provides[0].to_string(), "flare.console@1.0");
        assert!(m.requires.is_empty());
    }

    #[test]
    fn rejects_inconsistent_manifests() {
        let errs = parse(
            "[component]\nname = \"Bad_Name\"\nversion = \"1\"\nkind = \"driver\"\nswap = \"live\"\ncolour = 1\n",
        )
        .unwrap_err();
        let msgs: Vec<&str> = errs.iter().map(|e| e.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("must be 1-31 chars")), "{:?}", msgs);
        assert!(msgs.iter().any(|m| m.contains("MAJOR.MINOR.PATCH") || m.contains("minor")), "{:?}", msgs);
        assert!(msgs.iter().any(|m| m.contains("drivers must declare")), "{:?}", msgs);
        assert!(msgs.iter().any(|m| m.contains("requires the quiesce")), "{:?}", msgs);
        assert!(msgs.iter().any(|m| m.contains("unknown key 'colour'")), "{:?}", msgs);
    }
}
