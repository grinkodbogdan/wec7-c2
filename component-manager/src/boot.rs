//! Boot compositions (`system/*.boot.toml`): which components the system
//! starts and *exactly* which capabilities each one receives. A component
//! gets nothing that is not listed here.
//!
//! ```toml
//! name = "demo"
//! board = "qemu/virt-phone"
//! halt-when-finished = ["hello"]       # optional, for test systems
//!
//! [component.uart]                     # instance name
//! manifest = "drivers/common/uart-pl011"
//! grants = [
//!   "uart=mmio:uart0",                 # <local-name>=<kind>:<target>[:<rights>][:badge=<n>]
//!   "uart-irq=irq:uart0",
//!   "console=endpoint:console:recv",
//!   "klog=klog:read",
//! ]
//! restart = "on-failure"               # never | on-failure
//! max-restarts = 3
//! ```

use crate::manifest::{Kind, Manifest};
use crate::resolve;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use flare_abi::{Rights, CAP_NAME_LEN, MAX_CAPS};
use flare_config::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantKind {
    /// Map register window `reg` of a described device.
    Mmio {
        device: String,
        reg: usize,
    },
    /// Interrupt line `index` of a described device.
    Irq {
        device: String,
        index: usize,
    },
    Endpoint {
        name: String,
        rights: Rights,
        badge: u64,
    },
    Klog {
        rights: Rights,
    },
    /// `size` bytes of DMA-able memory.
    Dma {
        size: u64,
    },
    /// Shared memory `name` (physically contiguous, system lifetime): every
    /// grant of the same name maps the same pages. Holders may hand it to
    /// devices (`DMA_PADDR`), like DMA memory.
    Shm {
        name: String,
        size: u64,
    },
    /// Feed the Linux input device `name` (`/dev/input/eventN`).
    Input {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub local: String,
    pub kind: GrantKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootComponent {
    pub instance: String,
    /// Directory holding `component.toml`, relative to the repository root.
    pub manifest: String,
    pub grants: Vec<Grant>,
    pub restart: RestartPolicy,
    pub max_restarts: u32,
    pub line: u32,
}

/// The Linux personality and its init process (`[linux]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxConfig {
    /// argv of pid 1; `init[0]` is a path in the root filesystem.
    pub init: Vec<String>,
    pub env: Vec<String>,
    /// Linux root may manage components through /dev/flarectl.
    pub control: bool,
    /// Shared memory holding a `flare.display@1.0` framebuffer, exposed to
    /// Linux as the DRM/KMS device /dev/dri/card0.
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composition {
    pub name: String,
    pub board: String,
    /// Instance names, or `linux` for the Linux init process.
    pub halt_when_finished: Vec<String>,
    pub components: Vec<BootComponent>,
    pub linux: Option<LinuxConfig>,
}

impl Composition {
    /// Endpoint names in first-mention order.
    /// Shared-memory regions (name, size) in first-mention order.
    pub fn shm_regions(&self) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = Vec::new();
        for c in &self.components {
            for g in &c.grants {
                if let GrantKind::Shm { name, size } = &g.kind {
                    if !out.iter().any(|(n, _)| n == name) {
                        out.push((name.clone(), *size));
                    }
                }
            }
        }
        out
    }

    /// Linux input devices in first-mention order (event0, event1, ...).
    pub fn input_devices(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &self.components {
            for g in &c.grants {
                if let GrantKind::Input { name } = &g.kind {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
            }
        }
        out
    }

    pub fn endpoints(&self) -> Vec<String> {
        let mut eps: Vec<String> = Vec::new();
        for c in &self.components {
            for g in &c.grants {
                if let GrantKind::Endpoint { name, .. } = &g.kind {
                    if !eps.contains(name) {
                        eps.push(name.clone());
                    }
                }
            }
        }
        eps
    }
}

/// Parse `<local>=<kind>:<target>[:<rights>][:badge=<n>]`.
pub fn parse_grant(s: &str) -> Result<Grant, String> {
    let (local, spec) = s.split_once('=').ok_or_else(|| format!("grant '{}' must be <local-name>=<kind>:...", s))?;
    let local = local.trim();
    if local.is_empty()
        || local.len() >= CAP_NAME_LEN
        || !local.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("grant name '{}' must be 1-{} chars of [A-Za-z0-9_-]", local, CAP_NAME_LEN - 1));
    }
    let parts: Vec<&str> = spec.split(':').collect();
    let num = |v: &str| -> Result<u64, String> {
        if let Some(h) = v.strip_prefix("0x") { u64::from_str_radix(h, 16) } else { v.parse() }
            .map_err(|_| format!("grant '{}': invalid number '{}'", s, v))
    };
    let size_of = |v: &str| -> Result<u64, String> {
        match (v.strip_suffix('k'), v.strip_suffix('m')) {
            (Some(k), _) => Ok(num(k)? * 1024),
            (_, Some(m)) => Ok(num(m)? * 1024 * 1024),
            _ => num(v),
        }
    };
    let target_index = |t: &str| -> Result<(String, usize), String> {
        match t.split_once('#') {
            Some((d, i)) => Ok((d.to_string(), num(i)? as usize)),
            None => Ok((t.to_string(), 0)),
        }
    };
    let kind = match parts.as_slice() {
        ["mmio", dev] => {
            let (device, reg) = target_index(dev)?;
            GrantKind::Mmio { device, reg }
        }
        ["irq", dev] => {
            let (device, index) = target_index(dev)?;
            GrantKind::Irq { device, index }
        }
        ["endpoint", name, rest @ ..] if !rest.is_empty() && rest.len() <= 2 => {
            let rights = Rights::parse(rest[0]).ok_or_else(|| format!("grant '{}': bad rights '{}'", s, rest[0]))?;
            let badge = match rest.get(1) {
                None => 0,
                Some(b) => num(b.strip_prefix("badge=").ok_or_else(|| format!("grant '{}': expected badge=<n>", s))?)?,
            };
            GrantKind::Endpoint { name: name.to_string(), rights, badge }
        }
        ["dma", size] => GrantKind::Dma { size: size_of(size)? },
        ["shm", name, size] => GrantKind::Shm { name: name.to_string(), size: size_of(size)? },
        ["input", name] => GrantKind::Input { name: name.to_string() },
        ["klog", r] => GrantKind::Klog { rights: Rights::parse(r).ok_or_else(|| format!("grant '{}': bad rights '{}'", s, r))? },
        _ => {
            return Err(format!(
                "grant '{}' not understood; expected mmio:<dev>[#n], irq:<dev>[#n], endpoint:<name>:<rights>[:badge=n], klog:<rights>, dma:<size>, shm:<name>:<size> or input:<name>",
                s
            ))
        }
    };
    Ok(Grant { local: local.to_string(), kind })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    pub line: u32,
    pub message: String,
}

pub fn parse(src: &str) -> Result<Composition, Vec<Diag>> {
    let doc = flare_config::parse(src).map_err(|e| alloc::vec![Diag { line: e.line, message: e.message }])?;
    let mut errs = Vec::new();
    let mut err = |line: u32, m: String| errs.push(Diag { line, message: m });
    let root = doc.root();
    for e in &root.entries {
        if !["name", "board", "halt-when-finished"].contains(&e.key.as_str()) {
            err(e.line, format!("unknown key '{}'", e.key));
        }
    }
    let s = |k: &str| root.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    let name = s("name").unwrap_or_else(|| {
        err(1, "missing name = \"...\"".into());
        String::new()
    });
    let board = s("board").unwrap_or_else(|| {
        err(1, "missing board = \"<vendor>/<board>\"".into());
        String::new()
    });
    let halt_when_finished = match root.get("halt-when-finished") {
        None => Vec::new(),
        Some(v) => v.as_str_list().unwrap_or_else(|| {
            err(root.line_of("halt-when-finished"), "halt-when-finished must be an array of instance names".into());
            Vec::new()
        }),
    };
    let mut components = Vec::new();
    let mut linux = None;
    for sec in doc.named_sections() {
        if sec.name == "linux" {
            for e in &sec.entries {
                if !["init", "env", "control", "display"].contains(&e.key.as_str()) {
                    err(e.line, format!("unknown key '{}' in [linux]", e.key));
                }
            }
            let list = |k: &str| sec.get(k).and_then(|v| v.as_str_list());
            match list("init") {
                Some(init) if !init.is_empty() && init[0].starts_with('/') => {
                    let control = sec.get("control").and_then(|v| v.as_bool()).unwrap_or(false);
                    let display = sec.get("display").and_then(|v| v.as_str()).map(|s| s.to_string());
                    linux = Some(LinuxConfig { init, env: list("env").unwrap_or_default(), control, display })
                }
                _ => err(sec.line, "[linux] needs init = [\"/absolute/path\", args...]".into()),
            }
            continue;
        }
        let Some(instance) = sec.name.strip_prefix("component.") else {
            err(sec.line, format!("unknown section [{}]; expected [component.<instance>]", sec.name));
            continue;
        };
        for e in &sec.entries {
            if !["manifest", "grants", "restart", "max-restarts"].contains(&e.key.as_str()) {
                err(e.line, format!("unknown key '{}' in [{}]", e.key, sec.name));
            }
        }
        let manifest = match sec.get("manifest").and_then(|v| v.as_str()) {
            Some(m) => m.to_string(),
            None => {
                err(sec.line, format!("[{}] needs manifest = \"<path>\"", sec.name));
                continue;
            }
        };
        let mut grants = Vec::new();
        match sec.get("grants") {
            None => {}
            Some(Value::Array(a)) => {
                for v in a {
                    match v.as_str().map(parse_grant) {
                        Some(Ok(g)) => grants.push(g),
                        Some(Err(m)) => err(sec.line_of("grants"), m),
                        None => err(sec.line_of("grants"), "grants must be strings".into()),
                    }
                }
            }
            Some(_) => err(sec.line_of("grants"), "grants must be an array of strings".into()),
        }
        let restart = match sec.get("restart").map(|v| v.as_str()) {
            None | Some(Some("never")) => RestartPolicy::Never,
            Some(Some("on-failure")) => RestartPolicy::OnFailure,
            _ => {
                err(sec.line_of("restart"), "restart must be \"never\" or \"on-failure\"".into());
                RestartPolicy::Never
            }
        };
        let max_restarts = match sec.get("max-restarts") {
            None => {
                if restart == RestartPolicy::OnFailure {
                    3
                } else {
                    0
                }
            }
            Some(Value::Int(n)) if *n <= 100 => *n as u32,
            Some(_) => {
                err(sec.line_of("max-restarts"), "max-restarts must be an integer <= 100".into());
                0
            }
        };
        components.push(BootComponent {
            instance: instance.to_string(),
            manifest,
            grants,
            restart,
            max_restarts,
            line: sec.line,
        });
    }
    if errs.is_empty() {
        Ok(Composition { name, board, halt_when_finished, components, linux })
    } else {
        Err(errs)
    }
}

/// What validation needs to know about a described, enabled device.
#[derive(Debug, Clone)]
pub struct DeviceView {
    pub name: String,
    pub compatible: Vec<String>,
    pub regs: usize,
    pub irqs: usize,
}

/// Check a composition against its component manifests (same order as
/// `comp.components`) and the board's enabled devices.
pub fn validate(comp: &Composition, manifests: &[Manifest], devices: &[DeviceView]) -> Vec<String> {
    let mut errs = Vec::new();
    let mut instances = BTreeMap::new();
    let mut mmio_owner: BTreeMap<(String, usize), String> = BTreeMap::new();
    let mut irq_owner: BTreeMap<(String, usize), String> = BTreeMap::new();
    let mut ep_recv: BTreeMap<String, usize> = BTreeMap::new();
    let mut ep_send: BTreeMap<String, usize> = BTreeMap::new();
    let mut klog_readers = Vec::new();
    let mut input_owner: BTreeMap<String, String> = BTreeMap::new();
    if let Some(d) = comp.linux.as_ref().and_then(|l| l.display.as_ref()) {
        if !comp.shm_regions().iter().any(|(n, _)| n == d) {
            errs.push(format!(
                "[linux] display = '{}' names no shared memory (grant shm:{}:<size> to its driver)",
                d, d
            ));
        }
    }

    if comp.components.is_empty() && comp.linux.is_none() {
        errs.push("composition starts no components".to_string());
    }
    for (c, m) in comp.components.iter().zip(manifests) {
        let who = &c.instance;
        if instances.insert(c.instance.clone(), ()).is_some() {
            errs.push(format!("instance '{}' declared twice", who));
        }
        if c.instance.len() >= flare_abi::COMPONENT_NAME_LEN {
            errs.push(format!("instance name '{}' is too long", who));
        }
        if c.grants.len() > MAX_CAPS {
            errs.push(format!("'{}': {} grants exceed the {} capability slots", who, c.grants.len(), MAX_CAPS));
        }
        let mut locals = BTreeMap::new();
        for g in &c.grants {
            if locals.insert(g.local.clone(), ()).is_some() {
                errs.push(format!("'{}': grant name '{}' used twice", who, g.local));
            }
            match &g.kind {
                GrantKind::Mmio { device, reg } => match devices.iter().find(|d| &d.name == device) {
                    None => errs.push(format!("'{}': mmio grant names unknown or disabled device '{}'", who, device)),
                    Some(d) if *reg >= d.regs => {
                        errs.push(format!("'{}': device '{}' has no register window #{}", who, device, reg))
                    }
                    Some(d) => {
                        if let Some(prev) = mmio_owner.insert((device.clone(), *reg), who.clone()) {
                            errs.push(format!(
                                "device '{}' registers granted to both '{}' and '{}'; hardware access is exclusive",
                                device, prev, who
                            ));
                        }
                        if m.kind == Kind::Driver && !d.compatible.iter().any(|dc| m.binds.contains(dc)) {
                            errs.push(format!(
                                "'{}' ({}) does not bind device '{}' (compatible {:?})",
                                who, m.name, device, d.compatible
                            ));
                        }
                        if m.kind != Kind::Driver {
                            errs.push(format!(
                                "'{}' is a {:?}, only drivers may receive device registers",
                                who, m.kind
                            ));
                        }
                    }
                },
                GrantKind::Irq { device, index } => match devices.iter().find(|d| &d.name == device) {
                    None => errs.push(format!("'{}': irq grant names unknown or disabled device '{}'", who, device)),
                    Some(d) if *index >= d.irqs => {
                        errs.push(format!("'{}': device '{}' has no interrupt #{}", who, device, index))
                    }
                    Some(_) => {
                        if let Some(prev) = irq_owner.insert((device.clone(), *index), who.clone()) {
                            errs.push(format!(
                                "interrupt {}#{} granted to both '{}' and '{}'",
                                device, index, prev, who
                            ));
                        }
                    }
                },
                GrantKind::Endpoint { name, rights, badge } => {
                    if rights.0 == 0 || !rights.is_subset_of(Rights::SEND | Rights::RECV | Rights::GRANT) {
                        errs.push(format!("'{}': endpoint rights must be a non-empty subset of send+recv+grant", who));
                    }
                    if *badge != 0 && rights.contains(Rights::RECV) {
                        errs.push(format!("'{}': badges identify senders; a receive capability cannot carry one", who));
                    }
                    if rights.contains(Rights::RECV) {
                        *ep_recv.entry(name.clone()).or_default() += 1;
                    }
                    if rights.contains(Rights::SEND) {
                        *ep_send.entry(name.clone()).or_default() += 1;
                    }
                }
                GrantKind::Dma { size } => {
                    if m.kind != Kind::Driver {
                        errs.push(format!("'{}' is a {:?}, only drivers may receive DMA memory", who, m.kind));
                    }
                    if *size == 0 || *size > 1 << 20 {
                        errs.push(format!("'{}': DMA region must be 1 byte to 1 MiB", who));
                    }
                }
                GrantKind::Shm { name, size } => {
                    if *size == 0 || *size > 64 << 20 {
                        errs.push(format!("'{}': shared memory '{}' must be 1 byte to 64 MiB", who, name));
                    }
                    if let Some((_, s)) = comp.shm_regions().iter().find(|(n, _)| n == name) {
                        if s != size {
                            errs.push(format!("shared memory '{}' granted with different sizes", name));
                        }
                    }
                }
                GrantKind::Input { name } => {
                    if m.kind != Kind::Driver {
                        errs.push(format!("'{}' is a {:?}, only drivers may feed input devices", who, m.kind));
                    }
                    if comp.linux.is_none() {
                        errs.push(format!("'{}': input device '{}' needs the Linux personality", who, name));
                    }
                    if let Some(prev) = input_owner.insert(name.clone(), who.clone()) {
                        errs.push(format!("input device '{}' fed by both '{}' and '{}'", name, prev, who));
                    }
                }
                GrantKind::Klog { rights } => {
                    if rights.0 == 0 || !rights.is_subset_of(Rights::READ | Rights::WRITE) {
                        errs.push(format!("'{}': klog rights must be read and/or write", who));
                    }
                    if rights.contains(Rights::READ) {
                        klog_readers.push(who.clone());
                    }
                }
            }
        }
    }
    for ep in comp.endpoints() {
        if ep_recv.get(&ep).copied().unwrap_or(0) == 0 {
            errs.push(format!("endpoint '{}' has no receiver", ep));
        }
    }
    if klog_readers.len() > 1 {
        errs.push(format!("only one component may read the kernel log, found {:?}", klog_readers));
    }
    for h in &comp.halt_when_finished {
        if !instances.contains_key(h) && !(h == "linux" && comp.linux.is_some()) {
            errs.push(format!("halt-when-finished names unknown instance '{}'", h));
        }
    }
    // Several instances of one component (two input devices behind the same
    // driver) resolve as one component.
    let mut unique: Vec<Manifest> = Vec::new();
    for m in manifests {
        if !unique.iter().any(|u| u.name == m.name) {
            unique.push(m.clone());
        }
    }
    if let Err(rs) = resolve::resolve(&unique) {
        for r in rs {
            errs.push(format!("{}", r));
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SwapPolicy;
    use alloc::vec;

    const SRC: &str = r#"
name = "t"
board = "qemu/virt-phone"
halt-when-finished = ["app"]

[component.uart]
manifest = "drivers/common/uart-pl011"
grants = ["uart=mmio:uart0", "irq=irq:uart0", "console=endpoint:console:recv", "klog=klog:read"]
restart = "on-failure"

[component.app]
manifest = "apps/hello"
grants = ["console=endpoint:console:send:badge=0x10"]
"#;

    fn manifests() -> Vec<Manifest> {
        let mut uart = Manifest::new("uart-pl011", Kind::Driver, SwapPolicy::Restart, &["flare.console@1.0"], &[]);
        uart.binds = vec!["arm,pl011".into()];
        vec![uart, Manifest::new("hello", Kind::App, SwapPolicy::Restart, &[], &["flare.console@1.0"])]
    }

    fn devices() -> Vec<DeviceView> {
        vec![DeviceView { name: "uart0".into(), compatible: vec!["arm,pl011".into()], regs: 1, irqs: 1 }]
    }

    #[test]
    fn parses_shared_memory_and_input_grants() {
        assert_eq!(
            parse_grant("fb=shm:display:4m").unwrap().kind,
            GrantKind::Shm { name: "display".into(), size: 4 << 20 }
        );
        assert_eq!(parse_grant("dma=dma:8k").unwrap().kind, GrantKind::Dma { size: 8192 });
        assert_eq!(parse_grant("in=input:touchscreen").unwrap().kind, GrantKind::Input { name: "touchscreen".into() });
        let src = "name = \"t\"\nboard = \"b\"\n[component.d]\nmanifest = \"m\"\ngrants = [\"fb=shm:display:4m\"]\n[component.e]\nmanifest = \"m\"\ngrants = [\"fb=shm:display:4m\", \"x=shm:other:4k\"]\n[linux]\ninit = [\"/bin/sh\"]\ndisplay = \"display\"\n";
        let c = parse(src).unwrap();
        assert_eq!(c.shm_regions(), vec![("display".to_string(), 4 << 20), ("other".to_string(), 4096)]);
        assert_eq!(c.linux.unwrap().display.as_deref(), Some("display"));
    }

    #[test]
    fn parses_and_validates_a_composition() {
        let c = parse(SRC).unwrap();
        assert_eq!(c.components.len(), 2);
        assert_eq!(c.components[0].max_restarts, 3);
        assert_eq!(c.components[1].restart, RestartPolicy::Never);
        assert_eq!(
            c.components[1].grants[0].kind,
            GrantKind::Endpoint { name: "console".into(), rights: Rights::SEND, badge: 0x10 }
        );
        assert_eq!(c.endpoints(), vec!["console".to_string()]);
        let errs = validate(&c, &manifests(), &devices());
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn grant_syntax() {
        assert_eq!(parse_grant("r=mmio:gic#1").unwrap().kind, GrantKind::Mmio { device: "gic".into(), reg: 1 });
        assert!(parse_grant("x=endpoint:c").is_err());
        assert!(parse_grant("x=endpoint:c:fly").is_err());
        assert!(parse_grant("x=teleport:now").is_err());
        assert!(parse_grant("=klog:read").is_err());
        assert!(parse_grant("a-name-that-is-far-too-long=klog:read").is_err());
    }

    #[test]
    fn rejects_excess_authority() {
        let src = SRC.replace("[component.app]", "[component.app]\n").replace(
            "grants = [\"console=endpoint:console:send:badge=0x10\"]",
            "grants = [\"console=endpoint:console:recv:badge=1\", \"u=mmio:uart0\", \"k=klog:read\", \"g=mmio:gpio0\"]",
        );
        let c = parse(&src).unwrap();
        let errs = validate(&c, &manifests(), &devices());
        let has = |s: &str| errs.iter().any(|e| e.contains(s));
        assert!(has("cannot carry one"), "{:?}", errs);
        assert!(has("granted to both 'uart' and 'app'"), "{:?}", errs);
        assert!(has("only drivers may receive device registers"), "{:?}", errs);
        assert!(has("only one component may read the kernel log"), "{:?}", errs);
        assert!(has("unknown or disabled device 'gpio0'"), "{:?}", errs);
    }
}
