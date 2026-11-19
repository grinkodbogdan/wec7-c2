//! Kernel build script.
//!
//! The kernel contains no board knowledge of its own. At build time this
//! script:
//! 1. loads the boot composition (`FLARE_SYSTEM`, default
//!    `system/qemu-virt-phone.boot.toml`) and the board it names,
//! 2. validates the board's FHD and the composition (capability grants,
//!    exclusive hardware ownership, interface resolution),
//! 3. generates `platform_gen.rs` (hardware table), `boot_gen.rs` (boot
//!    components and their grants) and the linker script.
//!
//! Component images are taken from `FLARE_COMPONENTS_DIR`; `cargo xtask
//! build` sets this up.

use flare_cm::boot::{self, DeviceView, GrantKind, RestartPolicy};
use flare_fhd::Platform;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const GIB: u64 = 1 << 30;

fn fail(msg: &str) -> ! {
    eprintln!("\nerror: {}\n", msg);
    std::process::exit(1);
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap().to_path_buf();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for v in ["FLARE_SYSTEM", "FLARE_COMPONENTS_DIR"] {
        println!("cargo:rerun-if-env-changed={}", v);
    }
    println!("cargo:rerun-if-changed=build.rs");

    let system = std::env::var("FLARE_SYSTEM").unwrap_or_else(|_| "system/qemu-virt-phone.boot.toml".into());
    let system_path = if Path::new(&system).is_absolute() { PathBuf::from(&system) } else { root.join(&system) };
    println!("cargo:rerun-if-changed={}", system_path.display());
    let src = std::fs::read_to_string(&system_path)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {}", system_path.display(), e)));
    let comp = boot::parse(&src).unwrap_or_else(|errs| {
        let msgs: Vec<String> =
            errs.iter().map(|d| format!("{}:{}: {}", system_path.display(), d.line, d.message)).collect();
        fail(&msgs.join("\n"))
    });

    // Hardware.
    let platform = flare_fhd::load_board(&root.join("hardware"), &comp.board)
        .unwrap_or_else(|errs| fail(&errs.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")));
    for s in &platform.sources {
        println!("cargo:rerun-if-changed={}", s);
    }
    let errs = flare_fhd::validate(&platform);
    if !errs.is_empty() {
        fail(&errs.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"));
    }
    check_kernel_constraints(&platform);
    std::fs::write(out.join("platform_gen.rs"), flare_fhd::codegen::kernel_platform_rs(&platform)).unwrap();

    // Linker script: load the kernel 512 KiB into the first RAM region.
    let ram = platform.ram().next().unwrap();
    let base = ram.base + 0x8_0000;
    let ld = include_str!("kernel.ld.in").replace("@KERNEL_BASE@", &format!("{:#x}", base));
    std::fs::write(out.join("kernel.ld"), ld).unwrap();
    println!("cargo:rerun-if-changed=kernel.ld.in");
    println!("cargo:rustc-link-arg-bins=-T{}", out.join("kernel.ld").display());

    // Components.
    let mut manifests = Vec::new();
    for c in &comp.components {
        let p = root.join(&c.manifest).join("component.toml");
        println!("cargo:rerun-if-changed={}", p.display());
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| fail(&format!("cannot read {}: {}", p.display(), e)));
        let m = flare_cm::manifest::parse(&text).unwrap_or_else(|errs| {
            fail(
                &errs
                    .iter()
                    .map(|e| format!("{}:{}: {}", p.display(), e.line, e.message))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        });
        manifests.push(m);
    }
    let enabled: Vec<&flare_fhd::Node> = platform.enabled_nodes().collect();
    let views: Vec<DeviceView> = enabled
        .iter()
        .map(|n| DeviceView {
            name: n.name.clone(),
            compatible: n.compatible.clone(),
            regs: n.regs.len(),
            irqs: n.irqs.len(),
        })
        .collect();
    let errs = boot::validate(&comp, &manifests, &views);
    if !errs.is_empty() {
        fail(&format!("invalid boot composition {}:\n  {}", system_path.display(), errs.join("\n  ")));
    }

    let images = match std::env::var("FLARE_COMPONENTS_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => fail("FLARE_COMPONENTS_DIR is not set; build the kernel with `cargo xtask build`"),
    };
    let eps = comp.endpoints();
    let mut g = String::new();
    let _ = writeln!(g, "// @generated from {}; do not edit.", system_path.display());
    let _ = writeln!(g, "pub const SYSTEM_NAME: &str = {:?};", comp.name);
    let _ = writeln!(g, "pub const ENDPOINTS: &[&str] = &{:?};", eps);
    let halt: Vec<usize> = comp
        .halt_when_finished
        .iter()
        .map(|h| comp.components.iter().position(|c| &c.instance == h).unwrap())
        .collect();
    let _ = writeln!(g, "pub const HALT_WHEN_FINISHED: &[usize] = &{:?};", halt);
    let _ = writeln!(g, "pub static COMPONENTS: &[BootComponent] = &[");
    for (c, m) in comp.components.iter().zip(&manifests) {
        let image = images.join(&m.name);
        if !image.exists() {
            fail(&format!(
                "component image {} not found; build the components first (`cargo xtask build`)",
                image.display()
            ));
        }
        println!("cargo:rerun-if-changed={}", image.display());
        let _ = writeln!(g, "    BootComponent {{");
        let _ = writeln!(g, "        name: {:?},", c.instance);
        let _ = writeln!(g, "        image: include_bytes!({:?}),", image.display().to_string());
        let restart = match c.restart {
            RestartPolicy::Never => "Restart::Never",
            RestartPolicy::OnFailure => "Restart::OnFailure",
        };
        let _ = writeln!(g, "        restart: {}, max_restarts: {},", restart, c.max_restarts);
        let _ = writeln!(g, "        grants: &[");
        for gr in &c.grants {
            let dev_index = |d: &str| enabled.iter().position(|n| n.name == d).unwrap();
            let kind = match &gr.kind {
                GrantKind::Mmio { device, reg } => {
                    format!("GrantKind::Mmio {{ device: {}, reg: {} }}", dev_index(device), reg)
                }
                GrantKind::Irq { device, index } => {
                    format!("GrantKind::Irq {{ intid: {} }}", enabled[dev_index(device)].irqs[*index])
                }
                GrantKind::Endpoint { name, rights, badge } => format!(
                    "GrantKind::Endpoint {{ endpoint: {}, rights: Rights({}), badge: {:#x} }}",
                    eps.iter().position(|e| e == name).unwrap(),
                    rights.0,
                    badge
                ),
                GrantKind::Klog { rights } => format!("GrantKind::Klog {{ rights: Rights({}) }}", rights.0),
            };
            let _ = writeln!(g, "            Grant {{ name: {:?}, kind: {} }},", gr.local, kind);
        }
        let _ = writeln!(g, "        ],");
        let _ = writeln!(g, "    }},");
    }
    let _ = writeln!(g, "];");
    std::fs::write(out.join("boot_gen.rs"), g).unwrap();
}

/// Constraints of the M1 kernel's memory map (1 GiB identity blocks).
fn check_kernel_constraints(p: &Platform) {
    let user = flare_abi::layout::USER_REGION_BASE;
    let mut ram_blocks = Vec::new();
    for m in p.ram() {
        if m.base + m.size > user {
            fail(&format!(
                "memory.{} extends above {:#x}, which the M1 kernel reserves for component address spaces",
                m.name, user
            ));
        }
        ram_blocks.extend((m.base / GIB)..=((m.base + m.size - 1) / GIB));
    }
    for n in p.enabled_nodes() {
        for r in &n.regs {
            let (first, last) = (r.base / GIB, (r.base + r.size - 1) / GIB);
            if r.base + r.size > user {
                fail(&format!("{}.{} is above {:#x}; not yet supported by the M1 kernel", n.class, n.name, user));
            }
            if (first..=last).any(|b| ram_blocks.contains(&b)) {
                fail(&format!(
                    "{}.{} shares a 1 GiB block with RAM; the M1 kernel maps memory with 1 GiB blocks",
                    n.class, n.name
                ));
            }
        }
    }
    if !p.enabled_nodes().any(|n| n.class == "intc" && n.is_compatible("arm,gic-v2")) {
        fail("the M1 kernel needs an arm,gic-v2 interrupt controller");
    }
}
