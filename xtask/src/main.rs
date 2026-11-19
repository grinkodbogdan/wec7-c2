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
const LINUX_TARGET: &str = "aarch64-unknown-linux-musl";
/// Static busybox for arm64 from Alpine Linux, pinned by checksum.
const BUSYBOX_URL: &str = "https://dl-cdn.alpinelinux.org/alpine/v3.20/main/aarch64/busybox-static-1.36.1-r31.apk";
const BUSYBOX_SHA256: &str = "1d8e7a7fc2ed69bdb8eb7be9d962489c21a0f75b72d8a325437c8a09d4cfac70";
const BUSYBOX_APPLETS: &[&str] = &[
    "sh",
    "ash",
    "ls",
    "cat",
    "echo",
    "mkdir",
    "rmdir",
    "rm",
    "cp",
    "mv",
    "ln",
    "touch",
    "chmod",
    "pwd",
    "uname",
    "wc",
    "grep",
    "head",
    "tail",
    "sleep",
    "true",
    "false",
    "env",
    "date",
    "sort",
    "uniq",
    "find",
    "seq",
    "test",
    "[",
    "printf",
    "hostname",
    "id",
    "whoami",
    "dmesg",
    "free",
    "uptime",
    "clear",
    "cut",
    "tr",
    "od",
    "hexdump",
    "xargs",
    "which",
    "basename",
    "dirname",
    "stat",
    "df",
    "du",
    "vi",
    "more",
    "tee",
    "yes",
    "expr",
    "awk",
    "sed",
    "kill",
    "ps",
    "md5sum",
    "sha256sum",
    "base64",
    "nproc",
    "tty",
    "stty",
    "reset",
    "cal",
    "factor",
    "time",
    "watch",
    "timeout",
    "usleep",
];
/// Linux ABI conformance tests (C, from QRT's tests/linux), built with the
/// musl SDK in compat/linux/sdk and installed in /usr/libexec/flare-tests.
const LINUX_TESTS: &[&str] = &["hello", "memory", "procs", "threads", "signals", "events", "binder"];
const BUSYBOX_SBIN: &[&str] = &["init", "poweroff", "reboot", "halt"];
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
    "kernel: component 'intruder' faulted: data abort (permission fault) at 0xffffff8040080000",
    "kernel: restarting 'intruder' (1/1)",
    "intruder: incarnation 1 probing authority boundaries",
    "kernel: component 'intruder' retired",
    "kernel: component 'hello' exited",
    "linux: started init (pid 1): /bin/sh /etc/rc",
    "Flare OS — Linux personality",
    "rc: uname -a: Linux flare 6.6.0-flare",
    "supervisor: restarting 'uart' (incarnation 1)",
    "uart-pl011: console service online (incarnation 1, user-space driver)",
    "rc: still talking after the console driver restarted",
    "rc: arithmetic: 6 * 7 = 42",
    "rc: file round-trip: written by sh",
    "rc: pipeline: 3 words",
    "rc: memory: MemTotal:",
    "std-test: args = [\"hello\", \"from\", \"rc\"]",
    "std-test: HashMap with 1000 entries, 999^2 = 998001",
    "std-test: file round-trip: rust std on flare",
    "std-test: sqrt(2) = 1.414214",
    "std-test: slept >= 50 ms: true",
    "std-test: child said 'spawned through fork/exec' (status 0)",
    "std-test: ok",
    "rc: std-test exit status 0",
    "rc: slept 1 second",
    "events: ok",
    "memory: ok",
    "procs: ok",
    "signals: ok",
    "threads: 4 threads, counter 80000 (expected 80000), TLS private",
    "threads: ok",
    "dynamic: dlopen/dlsym: ok",
    "dynamic: ok",
    "binder: death notification ok",
    "binder: ok",
    "shell-42",
    "linux: init exited (status 0)",
    "flare: system halted",
];
/// Typed into the console during `qemu-test` (read by the interactive shell).
const TEST_INPUT: &[u8] = b"echo shell-$((40 + 2))\rexit\r";
const FORBIDDEN: &[&str] = &[
    "KERNEL PANIC",
    "UNEXPECTED",
    "must never print",
    "panic:",
    "killed by SIG",
    "unimplemented system call",
    ": FAILED",
];

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
    let rootfs = assemble_rootfs();
    let mut k = cargo();
    k.args(["build", "--target", TARGET, "-p", "flare-kernel"]);
    if o.release {
        k.arg("--release");
    }
    k.env("FLARE_SYSTEM", root().join(&o.system)).env("FLARE_COMPONENTS_DIR", &out).env("FLARE_ROOTFS", &rootfs);
    run(k);
    out.join("flare-kernel")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// Download (once) and verify the static busybox binary.
fn busybox() -> PathBuf {
    let dir = root().join("target").join("downloads");
    let bin = dir.join("busybox.static");
    if bin.exists() {
        return bin;
    }
    std::fs::create_dir_all(&dir).unwrap();
    let apk = dir.join("busybox-static.apk");
    println!("xtask: downloading busybox from {}", BUSYBOX_URL);
    let mut c = Command::new("curl");
    c.args(["-fsSL", "--retry", "3", "-o"]).arg(&apk).arg(BUSYBOX_URL);
    run(c);
    let out = Command::new("sha256sum").arg(&apk).output().unwrap_or_else(|e| die(&format!("sha256sum: {}", e)));
    let sum = String::from_utf8_lossy(&out.stdout);
    if !sum.starts_with(BUSYBOX_SHA256) {
        let _ = std::fs::remove_file(&apk);
        die(&format!("busybox checksum mismatch: {}", sum.trim()));
    }
    let extract = dir.join("busybox-extract");
    std::fs::create_dir_all(&extract).unwrap();
    let mut t = Command::new("tar");
    t.arg("xzf").arg(&apk).arg("-C").arg(&extract).arg("bin/busybox.static");
    let _ = t.status();
    std::fs::rename(extract.join("bin/busybox.static"), &bin)
        .unwrap_or_else(|e| die(&format!("busybox.static not found in package: {}", e)));
    bin
}

/// Build (once) the musl C toolchain for Linux test programs.
fn linux_sdk() -> PathBuf {
    let out = root().join("target").join("linux-sdk");
    if !out.join(".stamp").exists() {
        println!("xtask: building the musl SDK for Linux test programs");
        let mut c = Command::new(root().join("compat/linux/sdk/build.sh"));
        c.arg(&out);
        run(c);
    }
    out.join("bin/flare-a64-cc")
}

/// Assemble the Linux root filesystem: compat/linux/rootfs, busybox with
/// applet links, and the Linux test programs.
fn assemble_rootfs() -> PathBuf {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let rootfs = root().join("target").join("flare-rootfs");
    let _ = std::fs::remove_dir_all(&rootfs);
    copy_tree(&root().join("compat/linux/rootfs"), &rootfs);
    for d in ["bin", "sbin", "lib", "usr/bin", "usr/lib", "root", "tmp", "dev", "proc"] {
        std::fs::create_dir_all(rootfs.join(d)).unwrap();
    }
    let bb = rootfs.join("bin/busybox");
    std::fs::copy(busybox(), &bb).unwrap();
    std::fs::set_permissions(&bb, std::fs::Permissions::from_mode(0o755)).unwrap();
    for a in BUSYBOX_APPLETS {
        symlink("busybox", rootfs.join("bin").join(a)).unwrap();
    }
    for a in BUSYBOX_SBIN {
        symlink("../bin/busybox", rootfs.join("sbin").join(a)).unwrap();
    }
    let mut c = cargo();
    c.args(["build", "--release", "--target", LINUX_TARGET, "-p", "std-test"]);
    run(c);
    let dst = rootfs.join("usr/bin/std-test");
    std::fs::copy(root().join("target").join(LINUX_TARGET).join("release/std-test"), &dst).unwrap();
    let cc = linux_sdk();
    let tests = rootfs.join("usr/libexec/flare-tests");
    std::fs::create_dir_all(&tests).unwrap();
    for t in LINUX_TESTS {
        let mut c = Command::new(&cc);
        c.args(["-O2", "-o"]).arg(tests.join(t)).arg(root().join("compat/linux/tests").join(format!("{}.c", t)));
        c.arg("-lpthread");
        run(c);
    }
    // Dynamic linking: musl's loader, a shared library, a dlopen plugin.
    let dyn_cc = cc.with_file_name("flare-a64-cc-dyn");
    let sysroot = root().join("target/linux-sdk/sysroot");
    std::fs::create_dir_all(rootfs.join("lib")).unwrap();
    std::fs::copy(sysroot.join("usr/lib/libc.so"), rootfs.join("lib/ld-musl-aarch64.so.1")).unwrap();
    symlink("/lib/ld-musl-aarch64.so.1", rootfs.join("usr/lib/libc.so")).unwrap();
    let src = root().join("compat/linux/tests/dynamic");
    let lib = rootfs.join("usr/lib");
    for (out, file, extra) in
        [("libgreet.so", "libgreet.c", &["-shared"][..]), ("plugin.so", "plugin.c", &["-shared", "-lgreet"][..])]
    {
        let mut c = Command::new(&dyn_cc);
        c.args(["-O2", "-o"]).arg(lib.join(out)).arg(src.join(file)).arg("-L").arg(&lib).args(extra);
        run(c);
    }
    let mut c = Command::new(&dyn_cc);
    c.args(["-O2", "-fPIE", "-pie", "-o"]).arg(tests.join("dynamic")).arg(src.join("dynamic.c"));
    c.arg("-L").arg(&lib).args(["-lgreet", "-Wl,-rpath,/usr/lib"]);
    run(c);
    rootfs
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
    c.args([
        "-smp",
        &p.cpus.to_string(),
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        "stdio",
        "-no-reboot",
        "-kernel",
    ]);
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
    let mut stdin = child.stdin.take().unwrap();
    let mut typed = false;
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
        // Type into the interactive shell once it is up.
        if !typed && String::from_utf8_lossy(&output.lock().unwrap()).contains("flare:~# ") {
            let _ = stdin.write_all(TEST_INPUT);
            let _ = stdin.flush();
            typed = true;
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
