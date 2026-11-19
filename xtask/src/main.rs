//! `cargo xtask` — build, run and test Flare.
//!
//!   cargo xtask build   [--system <file>] [--debug]   build components + kernel
//!   cargo xtask run     [--system <file>] [--debug]   build and boot in QEMU
//!   cargo xtask test                                  host tests + QEMU boot test
//!   cargo xtask qemu-test [--system <file>]           QEMU boot test only
//!   cargo xtask fhd <vendor>/<board>                  validate and show a board
//!   cargo xtask cm resolve [--system <file>]          interface wiring + start order
//!   cargo xtask cm swap <instance> <manifest-dir> [--system <file>]
//!                                                     plan a hot-swap

use flare_cm::boot::Composition;
use flare_cm::manifest::Manifest;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TARGET: &str = "aarch64-unknown-none-softfloat";
const DEFAULT_SYSTEM: &str = "system/qemu-virt-phone.boot.toml";

/// Lines the reference system must print during `qemu-test`.
const EXPECTED: &[&str] = &[
    "Flare OS framekernel",
    "fhd: board 'qemu-virt-phone' on soc 'qemu-virt'",
    "kernel: started 'uart'",
    "uart-pl011: console service online (incarnation 0, user-space driver)",
    "hello: Hello from a native Flare component 'hello' (incarnation 0)",
    "hello: this line travelled through a derived capability",
    "hello: derived capability revoked -> E_NO_CAP as expected",
    "spinner: starting a CPU-bound loop that never yields",
    "intruder: write kernel log via the console capability -> denied (E_WRONG_TYPE)",
    "intruder: send on empty slot 9 -> denied (E_NO_CAP)",
    "intruder: derive RECV from SEND-only console cap -> denied (E_RIGHTS)",
    "intruder: bind an interrupt without an irq cap -> denied (E_NO_CAP)",
    "intruder: reply without a pending call -> denied (E_NO_REPLY)",
    "kernel: component 'intruder' faulted: data abort (permission fault) at 0x40080000",
    "kernel: restarting 'intruder' (1/1)",
    "intruder: incarnation 1 probing authority boundaries",
    "kernel: component 'intruder' retired",
    "hello: received input: flare",
    "kernel: component 'hello' exited",
    "flare: system halted",
];
const FORBIDDEN: &[&str] = &["KERNEL PANIC", "UNEXPECTED", "must never print", "panic:"];

struct Opts {
    system: String,
    release: bool,
    rest: Vec<String>,
}

fn parse_opts(args: &[String]) -> Opts {
    let mut o = Opts { system: DEFAULT_SYSTEM.to_string(), release: true, rest: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--system" => o.system = it.next().cloned().unwrap_or_else(|| die("--system needs a file")),
            "--debug" => o.release = false,
            _ => o.rest.push(a.clone()),
        }
    }
    o
}

fn die(msg: &str) -> ! {
    eprintln!("xtask: {}", msg);
    exit(1)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut c: Command) {
    let status = c.status().unwrap_or_else(|e| die(&format!("cannot run {:?}: {}", c, e)));
    if !status.success() {
        die(&format!("command failed: {:?}", c));
    }
}

fn load_composition(system: &str) -> (Composition, Vec<Manifest>) {
    let path = root().join(system);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| die(&format!("{}: {}", path.display(), e)));
    let comp = flare_cm::boot::parse(&src).unwrap_or_else(|errs| {
        for e in errs {
            eprintln!("{}:{}: {}", path.display(), e.line, e.message);
        }
        exit(1)
    });
    let manifests = comp.components.iter().map(|c| load_manifest(&c.manifest)).collect();
    (comp, manifests)
}

fn load_manifest(dir: &str) -> Manifest {
    let p = root().join(dir).join("component.toml");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| die(&format!("{}: {}", p.display(), e)));
    flare_cm::manifest::parse(&text).unwrap_or_else(|errs| {
        for e in errs {
            eprintln!("{}:{}: {}", p.display(), e.line, e.message);
        }
        exit(1)
    })
}

/// Build the components named by the composition, then the kernel.
fn build(o: &Opts) -> PathBuf {
    let (_, manifests) = load_composition(&o.system);
    let profile = if o.release { "release" } else { "debug" };
    let mut c = cargo();
    c.args(["build", "--target", TARGET]);
    if o.release {
        c.arg("--release");
    }
    let mut names: Vec<&str> = manifests.iter().map(|m| m.name.as_str()).collect();
    names.dedup();
    for n in &names {
        c.args(["-p", n]);
    }
    run(c);
    let out = root().join("target").join(TARGET).join(profile);
    let mut k = cargo();
    k.args(["build", "--target", TARGET, "-p", "flare-kernel"]);
    if o.release {
        k.arg("--release");
    }
    k.env("FLARE_SYSTEM", root().join(&o.system)).env("FLARE_COMPONENTS_DIR", &out);
    run(k);
    out.join("flare-kernel")
}

fn qemu_command(o: &Opts, kernel: &Path) -> Command {
    let (comp, _) = load_composition(&o.system);
    let p = flare_fhd::load_board(&root().join("hardware"), &comp.board).unwrap_or_else(|errs| {
        for e in errs {
            eprintln!("{}", e);
        }
        exit(1)
    });
    let machine = p
        .emulation_str("qemu-machine")
        .unwrap_or_else(|| die(&format!("board '{}' has no [emulation] qemu-machine", comp.board)));
    let mut c = Command::new("qemu-system-aarch64");
    c.args(["-machine", machine]);
    if let Some(cpu) = p.emulation_str("qemu-cpu") {
        c.args(["-cpu", cpu]);
    }
    if let Some(mib) = p.emulation_int("memory-mib") {
        c.args(["-m", &format!("{}M", mib)]);
    }
    c.args(["-smp", "1", "-display", "none", "-monitor", "none", "-serial", "stdio", "-no-reboot", "-kernel"]);
    c.arg(kernel);
    c
}

fn qemu_test(o: &Opts) -> bool {
    let kernel = build(o);
    println!("xtask: booting {} in QEMU", kernel.display());
    let mut child = qemu_command(o, &kernel)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| die(&format!("cannot start qemu-system-aarch64: {}", e)));
    // Typed input for the hello app; the UART applies back-pressure until
    // the driver has enabled reception.
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(b"flare\r");
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut stdout = child.stdout.take().unwrap();
    let sink = output.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = std::io::stdout().write_all(&buf[..n]);
            let _ = std::io::stdout().flush();
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut timed_out = false;
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(stdin);
    let _ = reader.join();
    let text = String::from_utf8_lossy(&output.lock().unwrap()).replace('\r', "");
    let mut ok = !timed_out;
    println!("\n---- qemu-test ----");
    if timed_out {
        println!("FAIL: QEMU did not halt within 60 s");
    }
    for e in EXPECTED {
        if !text.contains(e) {
            println!("FAIL: missing output: {}", e);
            ok = false;
        }
    }
    for f in FORBIDDEN {
        if text.contains(f) {
            println!("FAIL: forbidden output present: {}", f);
            ok = false;
        }
    }
    if ok {
        println!("PASS: {} expected lines seen, system halted cleanly", EXPECTED.len());
    }
    ok
}

fn fhd(args: &[String]) {
    let board = args.first().map(|s| s.as_str()).unwrap_or("qemu/virt-phone");
    let p = flare_fhd::load_board(&root().join("hardware"), board).unwrap_or_else(|errs| {
        for e in errs {
            eprintln!("{}", e);
        }
        exit(1)
    });
    let errs = flare_fhd::validate(&p);
    println!("board {} (soc {}, {}, {} cpus)", p.board, p.soc, p.arch, p.cpus);
    for m in &p.memory {
        println!(
            "  memory   {:<16} {:#012x} +{:#x}{}",
            m.name,
            m.base,
            m.size,
            if m.reserved { " (reserved)" } else { "" }
        );
    }
    for n in &p.nodes {
        let regs: Vec<String> = n.regs.iter().map(|r| format!("{:#x}+{:#x}", r.base, r.size)).collect();
        println!(
            "  {:<9}{:<16} {:<9} {:<28} regs [{}] irqs {:?}",
            n.class,
            n.name,
            if n.enabled() { "okay" } else { "disabled" },
            n.compatible.first().cloned().unwrap_or_default(),
            regs.join(", "),
            n.irqs
        );
    }
    if errs.is_empty() {
        println!("valid");
    } else {
        for e in errs {
            eprintln!("{}", e);
        }
        exit(1);
    }
}

fn cm(o: &Opts) {
    let (comp, manifests) = load_composition(&o.system);
    let res = flare_cm::resolve::resolve(&manifests).unwrap_or_else(|errs| {
        for e in errs {
            eprintln!("{}", e);
        }
        exit(1)
    });
    match o.rest.get(1).map(|s| s.as_str()) {
        Some("resolve") | None => {
            println!("system '{}' on board '{}'", comp.name, comp.board);
            println!("start order:");
            for &i in &res.start_order {
                let m = &manifests[i];
                println!(
                    "  {:<10} {} {} ({:?}, swap: {:?})",
                    comp.components[i].instance, m.name, m.version, m.kind, m.swap
                );
            }
            println!("interface wiring:");
            for e in &res.edges {
                println!(
                    "  {} -> {} via {}",
                    comp.components[e.client].instance, comp.components[e.provider].instance, e.interface
                );
            }
        }
        Some("swap") => {
            let (Some(instance), Some(dir)) = (o.rest.get(2), o.rest.get(3)) else {
                die("usage: cargo xtask cm swap <instance> <manifest-dir>")
            };
            let Some(idx) = comp.components.iter().position(|c| &c.instance == instance) else {
                die(&format!("no instance '{}'", instance))
            };
            let new = load_manifest(dir);
            match flare_cm::swap::plan_replacement(&manifests, &res, &manifests[idx].name, &new) {
                Ok(plan) => {
                    println!(
                        "replace '{}' with {} {}: disruption {:?}",
                        instance, new.name, new.version, plan.disruption
                    );
                    for (i, s) in plan.steps.iter().enumerate() {
                        println!("  {}. {:?}", i + 1, s);
                    }
                    println!("unaffected: {:?}", plan.unaffected);
                }
                Err(errs) => {
                    for e in errs {
                        eprintln!("refused: {}", e);
                    }
                    exit(1);
                }
            }
        }
        Some(other) => die(&format!("unknown cm command '{}'", other)),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else {
        eprintln!("usage: cargo xtask <build|run|test|qemu-test|fhd|cm> [options]");
        exit(2)
    };
    let o = parse_opts(&args[1..]);
    match cmd.as_str() {
        "build" => {
            let k = build(&o);
            println!("kernel: {}", k.display());
        }
        "run" => {
            let k = build(&o);
            let status = qemu_command(&o, &k).status().unwrap_or_else(|e| die(&format!("cannot start QEMU: {}", e)));
            exit(status.code().unwrap_or(1));
        }
        "qemu-test" => exit(if qemu_test(&o) { 0 } else { 1 }),
        "test" => {
            let mut c = cargo();
            c.arg("test");
            run(c);
            exit(if qemu_test(&o) { 0 } else { 1 });
        }
        "fhd" => fhd(&o.rest),
        "cm" => {
            let mut o = o;
            o.rest.insert(0, cmd.clone());
            cm(&o)
        }
        _ => {
            eprintln!("unknown command '{}'", cmd);
            exit(2)
        }
    }
}
