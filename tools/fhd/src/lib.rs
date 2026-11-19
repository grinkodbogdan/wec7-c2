//! Flare Hardware Description (FHD), format version 0.
//!
//! A port describes hardware instead of patching the kernel. FHD files come
//! in layers:
//!
//! * `hardware/soc/<vendor>/<soc>/platform.fhd` — SoC facts: CPUs, memory
//!   windows, interrupt controller, timers, on-chip peripherals.
//! * `hardware/boards/<vendor>/<board>/board.fhd` — one phone/board: which
//!   SoC it uses (`soc = "<vendor>/<soc>"`), fitted memory, which peripherals
//!   are enabled, board-level wiring and properties.
//!
//! The board layer is merged over the SoC layer key by key. The merged
//! [`Platform`] is validated and then used to generate the kernel's platform
//! table and to bind drivers to devices. See `docs/fhd.md`.

use flare_config::{Document, Section, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const FHD_VERSION: u64 = 0;
pub const SUPPORTED_ARCHES: &[&str] = &["arm64"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg {
    pub base: u64,
    pub size: u64,
}

impl Reg {
    pub fn end(&self) -> Option<u64> {
        self.base.checked_add(self.size)
    }
    pub fn overlaps(&self, other: &Reg) -> bool {
        match (self.end(), other.end()) {
            (Some(a), Some(b)) => self.base < b && other.base < a,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Okay,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemRegion {
    pub name: String,
    pub base: u64,
    pub size: u64,
    pub reserved: bool,
    pub origin: Origin,
}

/// A hardware node: `[<class>.<name>]`, e.g. `[device.uart0]`, `[intc.gic]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub class: String,
    pub name: String,
    /// Most specific first.
    pub compatible: Vec<String>,
    pub regs: Vec<Reg>,
    /// Interrupt-controller-native interrupt ids (GIC INTIDs on arm64).
    pub irqs: Vec<u32>,
    pub status: Status,
    /// Names of nodes this node depends on (regulators, clocks, buses...).
    pub requires: Vec<String>,
    /// Any further properties.
    pub props: BTreeMap<String, Value>,
    pub origin: Origin,
}

impl Node {
    pub fn enabled(&self) -> bool {
        self.status == Status::Okay
    }
    pub fn is_compatible(&self, compat: &str) -> bool {
        self.compatible.iter().any(|c| c == compat)
    }
}

/// The merged description of one board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub board: String,
    pub soc: String,
    pub arch: String,
    pub cpus: u32,
    pub description: Option<String>,
    pub memory: Vec<MemRegion>,
    pub nodes: Vec<Node>,
    pub console: Option<String>,
    /// Emulator settings for boards that are virtual (`[emulation]`).
    pub emulation: BTreeMap<String, Value>,
    /// Files this platform was assembled from (for build-system rerun triggers).
    pub sources: Vec<String>,
}

impl Platform {
    pub fn node(&self, name: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.name == name)
    }
    pub fn enabled_nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| n.enabled())
    }
    pub fn ram(&self) -> impl Iterator<Item = &MemRegion> {
        self.memory.iter().filter(|m| !m.reserved)
    }
    pub fn emulation_str(&self, key: &str) -> Option<&str> {
        self.emulation.get(key).and_then(|v| v.as_str())
    }
    pub fn emulation_int(&self, key: &str) -> Option<u64> {
        self.emulation.get(key).and_then(|v| v.as_int())
    }
}

pub struct SourceFile {
    pub path: String,
    pub text: String,
}

/// Load `hardware/boards/<board>/board.fhd` and the SoC layer it references.
pub fn load_board(hardware_root: &Path, board: &str) -> Result<Platform, Vec<Diagnostic>> {
    let board_path = hardware_root.join("boards").join(board).join("board.fhd");
    let board_file = read_source(&board_path).map_err(|m| vec![diag(&board_path.display().to_string(), 0, m)])?;
    load_from_sources(board_file, |soc| {
        let p: PathBuf = hardware_root.join("soc").join(soc).join("platform.fhd");
        read_source(&p)
    })
}

fn read_source(p: &Path) -> Result<SourceFile, String> {
    std::fs::read_to_string(p)
        .map(|text| SourceFile { path: p.display().to_string(), text })
        .map_err(|e| format!("cannot read {}: {}", p.display(), e))
}

fn diag(file: &str, line: u32, message: impl Into<String>) -> Diagnostic {
    Diagnostic { file: file.to_string(), line, message: message.into() }
}

struct Layer {
    file: String,
    doc: Document,
}

/// One merged key: value plus origin.
type Merged = BTreeMap<String, (Value, Origin)>;

/// Load a board from in-memory sources; `soc_loader` resolves the board's
/// `soc = "..."` reference.
pub fn load_from_sources(
    board: SourceFile,
    soc_loader: impl Fn(&str) -> Result<SourceFile, String>,
) -> Result<Platform, Vec<Diagnostic>> {
    let parse = |f: SourceFile| -> Result<Layer, Vec<Diagnostic>> {
        flare_config::parse(&f.text)
            .map(|doc| Layer { file: f.path.clone(), doc })
            .map_err(|e| vec![diag(&f.path, e.line, e.message)])
    };
    let board = parse(board)?;
    let mut errs = Vec::new();
    check_root(&board, "board", &mut errs);
    let soc_ref = match board.doc.root().get("soc").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            errs.push(diag(&board.file, 1, "board must name its SoC: soc = \"<vendor>/<soc>\""));
            return Err(errs);
        }
    };
    let soc = match soc_loader(&soc_ref) {
        Ok(f) => parse(f)?,
        Err(m) => {
            errs.push(diag(&board.file, board.doc.root().line_of("soc"), m));
            return Err(errs);
        }
    };
    check_root(&soc, "soc", &mut errs);

    // Merge sections: SoC first, board overrides key by key.
    let mut sections: Vec<(String, Merged, Origin)> = Vec::new();
    for layer in [&soc, &board] {
        for s in layer.doc.named_sections() {
            let origin = Origin { file: layer.file.clone(), line: s.line };
            let idx = match sections.iter().position(|(n, _, _)| *n == s.name) {
                Some(i) => i,
                None => {
                    sections.push((s.name.clone(), Merged::new(), origin));
                    sections.len() - 1
                }
            };
            for e in &s.entries {
                sections[idx]
                    .1
                    .insert(e.key.clone(), (e.value.clone(), Origin { file: layer.file.clone(), line: e.line }));
            }
        }
    }

    let root_str = |l: &Layer, k: &str| l.doc.root().get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    let mut platform = Platform {
        board: root_str(&board, "name").unwrap_or_default(),
        soc: root_str(&soc, "name").unwrap_or_default(),
        arch: root_str(&soc, "arch").unwrap_or_default(),
        cpus: soc.doc.root().get("cpus").and_then(|v| v.as_int()).unwrap_or(1) as u32,
        description: root_str(&board, "description"),
        memory: Vec::new(),
        nodes: Vec::new(),
        console: None,
        emulation: BTreeMap::new(),
        sources: vec![soc.file.clone(), board.file.clone()],
    };
    if board.doc.root().get("arch").is_some() {
        errs.push(diag(
            &board.file,
            board.doc.root().line_of("arch"),
            "the architecture is a SoC fact; set it in the SoC layer",
        ));
    }

    for (name, keys, origin) in sections {
        interpret_section(&name, keys, origin, &mut platform, &mut errs);
    }
    if errs.is_empty() {
        Ok(platform)
    } else {
        Err(errs)
    }
}

fn check_root(layer: &Layer, kind: &str, errs: &mut Vec<Diagnostic>) {
    let root: &Section = layer.doc.root();
    let allowed: &[&str] = if kind == "soc" {
        &["fhd", "kind", "name", "arch", "cpus", "description"]
    } else {
        &["fhd", "kind", "name", "soc", "description", "arch"]
    };
    for e in &root.entries {
        if !allowed.contains(&e.key.as_str()) {
            errs.push(diag(&layer.file, e.line, format!("unknown top-level key '{}'", e.key)));
        }
    }
    match root.get("fhd").and_then(|v| v.as_int()) {
        Some(FHD_VERSION) => {}
        Some(v) => errs.push(diag(
            &layer.file,
            root.line_of("fhd"),
            format!("unsupported FHD version {} (this tool reads version {})", v, FHD_VERSION),
        )),
        None => errs.push(diag(&layer.file, 1, "missing format version: fhd = 0")),
    }
    match root.get("kind").and_then(|v| v.as_str()) {
        Some(k) if k == kind => {}
        Some(k) => {
            errs.push(diag(&layer.file, root.line_of("kind"), format!("expected kind = \"{}\", found \"{}\"", kind, k)))
        }
        None => errs.push(diag(&layer.file, 1, format!("missing kind = \"{}\"", kind))),
    }
    if root.get("name").and_then(|v| v.as_str()).is_none() {
        errs.push(diag(&layer.file, 1, "missing name = \"...\""));
    }
    if kind == "soc" {
        match root.get("arch").and_then(|v| v.as_str()) {
            Some(a) if SUPPORTED_ARCHES.contains(&a) => {}
            Some(a) => errs.push(diag(
                &layer.file,
                root.line_of("arch"),
                format!("unsupported arch '{}' (supported: {:?})", a, SUPPORTED_ARCHES),
            )),
            None => errs.push(diag(&layer.file, 1, "missing arch = \"...\"")),
        }
    }
}

fn interpret_section(name: &str, mut keys: Merged, origin: Origin, p: &mut Platform, errs: &mut Vec<Diagnostic>) {
    let mut err = |o: &Origin, m: String| errs.push(diag(&o.file, o.line, m));
    match name {
        "chosen" => {
            for (k, (v, o)) in keys {
                match (k.as_str(), v.as_str()) {
                    ("console", Some(c)) => p.console = Some(c.to_string()),
                    ("console", None) => err(&o, "chosen.console must be a node name string".into()),
                    _ => err(&o, format!("unknown key '{}' in [chosen]", k)),
                }
            }
            return;
        }
        "emulation" => {
            p.emulation = keys.into_iter().map(|(k, (v, _))| (k, v)).collect();
            return;
        }
        _ => {}
    }
    let Some((class, node_name)) = name.split_once('.') else {
        err(&origin, format!("unknown section [{}]; hardware nodes are written [<class>.<name>]", name));
        return;
    };
    if node_name.contains('.') {
        err(&origin, format!("node name '{}' must not contain '.'", node_name));
        return;
    }
    if class == "memory" {
        let mut int = |k: &str| match keys.remove(k) {
            Some((Value::Int(i), _)) => Some(i),
            Some((v, o)) => {
                err(&o, format!("memory.{}: '{}' must be an integer, found {}", node_name, k, v.type_name()));
                None
            }
            None => {
                err(&origin, format!("memory.{}: missing '{}'", node_name, k));
                None
            }
        };
        let base = int("base");
        let size = int("size");
        let reserved = match keys.remove("reserved") {
            Some((Value::Bool(b), _)) => b,
            Some((_, o)) => {
                err(&o, "reserved must be true or false".into());
                false
            }
            None => false,
        };
        for (k, (_, o)) in keys {
            err(&o, format!("unknown key '{}' in memory region", k));
        }
        if let (Some(base), Some(size)) = (base, size) {
            p.memory.push(MemRegion { name: node_name.to_string(), base, size, reserved, origin });
        }
        return;
    }

    let mut node = Node {
        class: class.to_string(),
        name: node_name.to_string(),
        compatible: Vec::new(),
        regs: Vec::new(),
        irqs: Vec::new(),
        status: Status::Okay,
        requires: Vec::new(),
        props: BTreeMap::new(),
        origin,
    };
    for (k, (v, o)) in keys {
        match k.as_str() {
            "compatible" => match v.as_str_list() {
                Some(l) => node.compatible = l,
                None => err(&o, "compatible must be a string or array of strings".into()),
            },
            "reg" => match parse_regs(&v) {
                Some(r) => node.regs = r,
                None => err(&o, "reg must be [base, size] pairs: [0x1000, 0x100] or [[0x1000, 0x100], ...]".into()),
            },
            "interrupts" => match v.as_array().map(|a| {
                a.iter()
                    .map(|x| x.as_int().filter(|&i| i <= u32::MAX as u64).map(|i| i as u32))
                    .collect::<Option<Vec<_>>>()
            }) {
                Some(Some(irqs)) => node.irqs = irqs,
                _ => err(&o, "interrupts must be an array of interrupt numbers".into()),
            },
            "status" => match v.as_str() {
                Some("okay") => node.status = Status::Okay,
                Some("disabled") => node.status = Status::Disabled,
                _ => err(&o, "status must be \"okay\" or \"disabled\"".into()),
            },
            "requires" => match v.as_str_list() {
                Some(l) => node.requires = l,
                None => err(&o, "requires must be an array of node names".into()),
            },
            _ => {
                node.props.insert(k, v);
            }
        }
    }
    p.nodes.push(node);
}

fn parse_regs(v: &Value) -> Option<Vec<Reg>> {
    let a = v.as_array()?;
    if a.iter().all(|x| x.as_int().is_some()) {
        if a.len() % 2 != 0 || a.is_empty() {
            return None;
        }
        return Some(a.chunks(2).map(|c| Reg { base: c[0].as_int().unwrap(), size: c[1].as_int().unwrap() }).collect());
    }
    a.iter()
        .map(|pair| match pair.as_array() {
            Some([Value::Int(b), Value::Int(s)]) => Some(Reg { base: *b, size: *s }),
            _ => None,
        })
        .collect()
}

/// Semantic validation of a merged platform. Returns all problems found.
pub fn validate(p: &Platform) -> Vec<Diagnostic> {
    let mut errs = Vec::new();
    let mut err = |o: &Origin, m: String| errs.push(diag(&o.file, o.line, m));

    if p.ram().all(|m| m.size == 0) {
        let o = Origin { file: p.sources.last().cloned().unwrap_or_default(), line: 1 };
        err(&o, "no usable (non-reserved) memory region".into());
    }
    for m in &p.memory {
        let r = Reg { base: m.base, size: m.size };
        if m.size == 0 || r.end().is_none() {
            err(&m.origin, format!("memory.{}: empty or overflowing region", m.name));
        }
    }
    // Every enabled node's registers: sane, and not overlapping anything else.
    let mut claimed: Vec<(String, Reg, Origin)> = p
        .memory
        .iter()
        .map(|m| (format!("memory.{}", m.name), Reg { base: m.base, size: m.size }, m.origin.clone()))
        .collect();
    for n in p.enabled_nodes() {
        if n.compatible.is_empty() {
            err(&n.origin, format!("{}.{}: enabled node needs a compatible string", n.class, n.name));
        }
        for r in &n.regs {
            if r.size == 0 || r.end().is_none() {
                err(
                    &n.origin,
                    format!("{}.{}: empty or overflowing register window {:#x}+{:#x}", n.class, n.name, r.base, r.size),
                );
                continue;
            }
            if let Some((other, _, _)) = claimed.iter().find(|(_, c, _)| c.overlaps(r)) {
                err(
                    &n.origin,
                    format!("{}.{}: register window {:#x}+{:#x} overlaps {}", n.class, n.name, r.base, r.size, other),
                );
            }
            claimed.push((format!("{}.{}", n.class, n.name), *r, n.origin.clone()));
        }
        for req in &n.requires {
            match p.node(req) {
                None => err(&n.origin, format!("{}.{}: requires unknown node '{}'", n.class, n.name, req)),
                Some(r) if !r.enabled() => {
                    err(&n.origin, format!("{}.{}: requires '{}', which is disabled", n.class, n.name, req))
                }
                _ => {}
            }
        }
    }
    let intcs: Vec<&Node> = p.enabled_nodes().filter(|n| n.class == "intc").collect();
    if intcs.len() != 1 {
        let o = Origin { file: p.sources.first().cloned().unwrap_or_default(), line: 1 };
        err(&o, format!("expected exactly one enabled interrupt controller ([intc.*]), found {}", intcs.len()));
    }
    if !p.enabled_nodes().any(|n| n.class == "timer" && !n.irqs.is_empty()) {
        let o = Origin { file: p.sources.first().cloned().unwrap_or_default(), line: 1 };
        err(&o, "no enabled timer ([timer.*]) with an interrupt".into());
    }
    // Interrupt lines are exclusive unless explicitly shared.
    let mut seen: BTreeMap<u32, String> = BTreeMap::new();
    for n in p.enabled_nodes() {
        let shared = n.props.get("shared-irq").and_then(|v| v.as_bool()).unwrap_or(false);
        for &irq in &n.irqs {
            if let Some(prev) = seen.get(&irq) {
                if !shared {
                    err(&n.origin, format!("{}.{}: interrupt {} already used by {}", n.class, n.name, irq, prev));
                }
            } else {
                seen.insert(irq, format!("{}.{}", n.class, n.name));
            }
        }
    }
    if let Some(c) = &p.console {
        match p.node(c) {
            Some(n) if n.enabled() => {}
            Some(n) => err(&n.origin, format!("chosen.console '{}' is disabled", c)),
            None => {
                let o = Origin { file: p.sources.last().cloned().unwrap_or_default(), line: 1 };
                err(&o, format!("chosen.console refers to unknown node '{}'", c))
            }
        }
    }
    if let Some(mib) = p.emulation_int("memory-mib") {
        let ram: u64 = p.ram().map(|m| m.size).sum();
        if mib * 1024 * 1024 != ram {
            let o = Origin { file: p.sources.last().cloned().unwrap_or_default(), line: 1 };
            err(
                &o,
                format!("emulation.memory-mib = {} disagrees with described RAM ({} MiB)", mib, ram / (1024 * 1024)),
            );
        }
    }
    errs
}

/// Code generation for consumers of an FHD.
pub mod codegen {
    use super::*;
    use std::fmt::Write;

    /// Rust source for the kernel's `platform` module. Expects the including
    /// module to define `MemRange` and `DeviceDesc`.
    pub fn kernel_platform_rs(p: &Platform) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "// @generated by flare-fhd from: {}", p.sources.join(", "));
        let _ = writeln!(s, "// Do not edit; change the hardware description instead.");
        let _ = writeln!(s, "pub const BOARD: &str = {:?};", p.board);
        let _ = writeln!(s, "pub const SOC: &str = {:?};", p.soc);
        let _ = writeln!(s, "pub const ARCH: &str = {:?};", p.arch);
        let _ = writeln!(s, "pub const CPUS: u32 = {};", p.cpus);
        let ranges = |it: &mut dyn Iterator<Item = &MemRegion>| {
            it.map(|m| format!("MemRange {{ base: {:#x}, size: {:#x} }}", m.base, m.size))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ =
            writeln!(s, "pub const RAM: &[MemRange] = &[{}];", ranges(&mut p.memory.iter().filter(|m| !m.reserved)));
        let _ = writeln!(
            s,
            "pub const RESERVED: &[MemRange] = &[{}];",
            ranges(&mut p.memory.iter().filter(|m| m.reserved))
        );
        let enabled: Vec<&Node> = p.enabled_nodes().collect();
        let _ = writeln!(s, "pub const DEVICES: &[DeviceDesc] = &[");
        for n in &enabled {
            let regs: Vec<String> =
                n.regs.iter().map(|r| format!("MemRange {{ base: {:#x}, size: {:#x} }}", r.base, r.size)).collect();
            let strs: Vec<String> =
                n.props.iter().filter_map(|(k, v)| v.as_str().map(|sv| format!("({:?}, {:?})", k, sv))).collect();
            let ints: Vec<String> =
                n.props.iter().filter_map(|(k, v)| v.as_int().map(|iv| format!("({:?}, {:#x})", k, iv))).collect();
            let _ = writeln!(
                s,
                "    DeviceDesc {{ name: {:?}, class: {:?}, compatible: &{:?}, regs: &[{}], irqs: &{:?}, str_props: &[{}], int_props: &[{}] }},",
                n.name,
                n.class,
                n.compatible,
                regs.join(", "),
                n.irqs,
                strs.join(", "),
                ints.join(", ")
            );
        }
        let _ = writeln!(s, "];");
        let console = p.console.as_ref().and_then(|c| enabled.iter().position(|n| &n.name == c));
        let _ = writeln!(s, "pub const CONSOLE: Option<usize> = {:?};", console);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOC: &str = r#"
fhd = 0
kind = "soc"
name = "test-soc"
arch = "arm64"
cpus = 2

[memory.dram]
base = 0x40000000
size = 0x40000000

[intc.gic]
compatible = "arm,gic-v2"
reg = [[0x08000000, 0x10000], [0x08010000, 0x10000]]

[timer.t]
compatible = "arm,armv8-timer"
interrupts = [27]

[device.uart0]
compatible = "arm,pl011"
reg = [0x09000000, 0x1000]
interrupts = [33]

[device.spi0]
compatible = "vendor,spi"
reg = [0x09100000, 0x1000]
interrupts = [40]
status = "disabled"
"#;

    fn board(extra: &str) -> SourceFile {
        SourceFile {
            path: "board.fhd".into(),
            text: format!("fhd = 0\nkind = \"board\"\nname = \"test-phone\"\nsoc = \"test/soc\"\n{}", extra),
        }
    }

    fn load(extra: &str) -> Result<Platform, Vec<Diagnostic>> {
        load_from_sources(board(extra), |r| {
            assert_eq!(r, "test/soc");
            Ok(SourceFile { path: "soc.fhd".into(), text: SOC.into() })
        })
    }

    #[test]
    fn board_layer_overrides_soc_layer() {
        let p = load("[memory.dram]\nsize = 0x10000000\n[device.spi0]\nstatus = \"okay\"\npanel = \"example,panel\"\n[chosen]\nconsole = \"uart0\"\n").unwrap();
        assert_eq!(p.board, "test-phone");
        assert_eq!(p.soc, "test-soc");
        assert_eq!(p.memory[0].base, 0x40000000);
        assert_eq!(p.memory[0].size, 0x10000000);
        let spi = p.node("spi0").unwrap();
        assert!(spi.enabled());
        assert_eq!(spi.props.get("panel").unwrap().as_str(), Some("example,panel"));
        assert_eq!(spi.origin.file, "soc.fhd");
        assert!(validate(&p).is_empty(), "{:?}", validate(&p));
        let code = codegen::kernel_platform_rs(&p);
        assert!(code.contains("pub const CONSOLE: Option<usize> = Some(2);"), "{}", code);
        assert!(code.contains("(\"panel\", \"example,panel\")"));
    }

    #[test]
    fn detects_overlaps_irq_conflicts_and_bad_references() {
        let p = load(
            "[device.evil]\ncompatible = \"x\"\nreg = [0x09000800, 0x1000]\ninterrupts = [33]\nrequires = [\"spi0\", \"nope\"]\n[chosen]\nconsole = \"missing\"\n",
        )
        .unwrap();
        let msgs: Vec<String> = validate(&p).into_iter().map(|d| d.message).collect();
        let has = |s: &str| msgs.iter().any(|m| m.contains(s));
        assert!(has("overlaps device.uart0"), "{:?}", msgs);
        assert!(has("interrupt 33 already used"), "{:?}", msgs);
        assert!(has("requires 'spi0', which is disabled"), "{:?}", msgs);
        assert!(has("unknown node 'nope'"), "{:?}", msgs);
        assert!(has("chosen.console refers to unknown node"), "{:?}", msgs);
    }

    #[test]
    fn structural_errors_carry_file_and_line() {
        let errs = load("arch = \"riscv64\"\nbogus = 1\n[device.x]\nreg = [1, 2, 3]\n").unwrap_err();
        assert!(
            errs.iter().any(|d| d.file == "board.fhd" && d.line == 5 && d.message.contains("SoC fact")),
            "{:?}",
            errs
        );
        assert!(errs.iter().any(|d| d.line == 6 && d.message.contains("unknown top-level key 'bogus'")));
        assert!(errs.iter().any(|d| d.line == 8 && d.message.contains("reg must be")));
    }

    #[test]
    fn emulation_memory_must_match() {
        let p = load("[memory.dram]\nsize = 0x10000000\n[emulation]\nmemory-mib = 512\n").unwrap();
        assert!(validate(&p).iter().any(|d| d.message.contains("memory-mib")));
    }

    #[test]
    fn checked_in_hardware_descriptions_are_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hardware");
        let boards = root.join("boards");
        let mut count = 0;
        for vendor in std::fs::read_dir(&boards).unwrap() {
            for b in std::fs::read_dir(vendor.unwrap().path()).unwrap() {
                let b = b.unwrap().path();
                let rel = b.strip_prefix(&boards).unwrap().to_str().unwrap().to_string();
                let p = load_board(&root, &rel).unwrap_or_else(|e| panic!("{}: {:?}", rel, e));
                let errs = validate(&p);
                assert!(errs.is_empty(), "{}: {:?}", rel, errs);
                count += 1;
            }
        }
        assert!(count > 0);
    }
}
