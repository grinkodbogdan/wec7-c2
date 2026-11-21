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


fn fail(msg: &str) -> ! {
    eprintln!("\nerror: {}\n", msg);
    std::process::exit(1);
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap().to_path_buf();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for v in ["FLARE_SYSTEM", "FLARE_COMPONENTS_DIR", "FLARE_ROOTFS"] {
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
    let ram = platform.ram().min_by_key(|m| m.base).unwrap();
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
    let mut errs = boot::validate(&comp, &manifests, &views);
    // Linux's hardware must not share a page with registers granted to a
    // component, even when the description splits a block into devices (a
    // GPIO controller and the pins of one key).
    let page = |b: u64, s: u64| (b & !0xfff, (b + s.max(1) + 0xfff) & !0xfff);
    for d in comp.linux.iter().flat_map(|l| l.hardware.iter()) {
        let Some(n) = enabled.iter().find(|n| &n.name == d) else { continue };
        for c in &comp.components {
            for gr in &c.grants {
                let GrantKind::Mmio { device, reg } = &gr.kind else { continue };
                let Some(o) = enabled.iter().find(|n| &n.name == device) else { continue };
                let Some(r) = o.regs.get(*reg) else { continue };
                let (a0, a1) = page(r.base, r.size);
                for lr in &n.regs {
                    let (b0, b1) = page(lr.base, lr.size);
                    if a0 < b1 && b0 < a1 {
                        errs.push(format!(
                            "'{}' (given to Linux) overlaps '{}', whose registers '{}' has; hardware access is exclusive",
                            d, device, c.instance
                        ));
                    }
                }
            }
        }
    }
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
    let shm = comp.shm_regions();
    let inputs = comp.input_devices();
    let _ = writeln!(g, "pub const SHM_REGIONS: &[(&str, u64)] = &{:?};", shm);
    let _ = writeln!(g, "pub const INPUT_DEVICES: &[&str] = &{:?};", inputs);
    let serials = comp.serial_devices();
    let _ = writeln!(g, "pub const SERIAL_DEVICES: &[&str] = &{:?};", serials);
    let display =
        comp.linux.as_ref().and_then(|l| l.display.as_ref()).and_then(|d| shm.iter().position(|(n, _)| n == d));
    let _ = writeln!(g, "pub const LINUX_DISPLAY: Option<usize> = {:?};", display);
    let halt: Vec<usize> = comp
        .halt_when_finished
        .iter()
        .filter(|h| *h != "linux")
        .map(|h| comp.components.iter().position(|c| &c.instance == h).unwrap())
        .collect();
    let _ = writeln!(g, "pub const HALT_WHEN_FINISHED: &[usize] = &{:?};", halt);
    let _ =
        writeln!(g, "pub const HALT_ON_LINUX_EXIT: bool = {};", comp.halt_when_finished.iter().any(|h| h == "linux"));
    match &comp.linux {
        Some(l) => {
            let rootfs = match std::env::var("FLARE_ROOTFS") {
                Ok(d) => PathBuf::from(d),
                Err(_) => fail("the composition starts Linux but FLARE_ROOTFS is not set; use `cargo xtask build`"),
            };
            let mut archive = Vec::new();
            let mut count = 0;
            pack_dir(&rootfs, &rootfs, &mut archive, &mut count);
            std::fs::write(out.join("initramfs.bin"), &archive).unwrap();
            let _ = writeln!(g, "pub const LINUX_INIT: Option<&[&str]> = Some(&{:?});", l.init);
            let _ = writeln!(g, "pub const LINUX_ENV: &[&str] = &{:?};", l.env);
            let _ = writeln!(g, "pub const LINUX_CONTROL: bool = {};", l.control);
            let _ = writeln!(g, "pub const LINUX_HARDWARE: &[&str] = &{:?};", l.hardware);
            let _ = writeln!(g, "pub const LINUX_SMC: &[(u32, u32)] = &{:?};", l.smc);
            // The full root filesystem in the boot image's ramdisk, at the
            // board's ramdisk address.
            let int = |k: &str| platform.boot_image.get(k).and_then(|v| v.as_int());
            let ramdisk = match (l.rootfs.as_str(), int("base"), int("ramdisk-offset")) {
                ("ramdisk", Some(b), Some(o)) => Some(b as u64 + o as u64),
                ("ramdisk", _, _) => fail("[linux] rootfs = \"ramdisk\" needs [boot-image] base and ramdisk-offset in the board's FHD"),
                _ => None,
            };
            let _ = writeln!(g, "pub const RAMDISK_PHYS: Option<u64> = {:?};", ramdisk);
            let li: Vec<usize> = l.inputs.iter().filter_map(|n| inputs.iter().position(|m| m == n)).collect();
            let _ = writeln!(g, "pub const LINUX_INPUTS: &[usize] = &{:?};", li);
            let _ = writeln!(
                g,
                "pub static INITRAMFS: &[u8] = include_bytes!({:?});",
                out.join("initramfs.bin").display().to_string()
            );
            println!(
                "cargo:warning=initramfs: {} entries, {} KiB from {}",
                count,
                archive.len() / 1024,
                rootfs.display()
            );
        }
        None => {
            let _ = writeln!(g, "pub const LINUX_INIT: Option<&[&str]> = None;");
            let _ = writeln!(g, "pub const LINUX_ENV: &[&str] = &[];");
            let _ = writeln!(g, "pub const LINUX_CONTROL: bool = false;");
            let _ = writeln!(g, "pub const LINUX_HARDWARE: &[&str] = &[];");
            let _ = writeln!(g, "pub const LINUX_SMC: &[(u32, u32)] = &[];");
            let _ = writeln!(g, "pub const LINUX_INPUTS: &[usize] = &[];");
            let _ = writeln!(g, "pub const RAMDISK_PHYS: Option<u64> = None;");
            let _ = writeln!(g, "pub static INITRAMFS: &[u8] = &[];");
        }
    }
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
                GrantKind::Dma { size } => format!("GrantKind::Dma {{ size: {:#x} }}", size),
                GrantKind::Shm { name, .. } => {
                    format!("GrantKind::Shm {{ region: {} }}", shm.iter().position(|(n, _)| n == name).unwrap())
                }
                GrantKind::Input { name } => {
                    format!("GrantKind::Input {{ device: {} }}", inputs.iter().position(|n| n == name).unwrap())
                }
                GrantKind::Serial { name } => {
                    format!("GrantKind::Serial {{ device: {} }}", serials.iter().position(|n| n == name).unwrap())
                }
            };
            let _ = writeln!(g, "            Grant {{ name: {:?}, kind: {} }},", gr.local, kind);
        }
        let _ = writeln!(g, "        ],");
        let _ = writeln!(g, "    }},");
    }
    let _ = writeln!(g, "];");
    std::fs::write(out.join("boot_gen.rs"), g).unwrap();
}

/// Append a directory tree to an initramfs archive (format in
/// `kernel/src/linux/vfs.rs`).
fn pack_dir(root: &Path, dir: &Path, out: &mut Vec<u8>, count: &mut usize) {
    use std::os::unix::fs::PermissionsExt;
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {}", dir.display(), e)))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        let rel = format!("/{}", p.strip_prefix(root).unwrap().display());
        let meta = std::fs::symlink_metadata(&p).unwrap();
        let (kind, data): (u8, Vec<u8>) = if meta.file_type().is_symlink() {
            (3, std::fs::read_link(&p).unwrap().to_string_lossy().into_owned().into_bytes())
        } else if meta.is_dir() {
            (2, Vec::new())
        } else {
            println!("cargo:rerun-if-changed={}", p.display());
            (1, std::fs::read(&p).unwrap())
        };
        out.push(kind);
        out.extend_from_slice(&(meta.permissions().mode() & 0o7777).to_le_bytes());
        out.extend_from_slice(&(rel.len() as u16).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(rel.as_bytes());
        out.extend_from_slice(&data);
        *count += 1;
        if kind == 2 {
            pack_dir(root, &p, out, count);
        }
    }
}

/// Constraints of the kernel's direct map (1 GiB blocks, 39-bit window).
fn check_kernel_constraints(p: &Platform) {
    let top: u64 = 1 << 39;
    for m in &p.memory {
        if m.base + m.size > top {
            fail(&format!("memory.{} extends above {:#x}, beyond the kernel's direct map", m.name, top));
        }
    }
    for n in p.enabled_nodes() {
        for r in &n.regs {
            if r.base + r.size > top {
                fail(&format!("{}.{} is above {:#x}, beyond the kernel's direct map", n.class, n.name, top));
            }
        }
    }
    let first = p.ram().map(|m| m.base).min().unwrap_or(0);
    if first % (2 << 20) != 0 {
        fail("the lowest RAM region must start on a 2 MiB boundary (the kernel loads at its base + 512 KiB)");
    }
    if !p.enabled_nodes().any(|n| n.class == "intc" && n.is_compatible("arm,gic-v2")) {
        fail("the kernel needs an arm,gic-v2 interrupt controller");
    }
}
