//! `cargo xtask` — build, run and test Flare.
//!
//!   cargo xtask build   [--system <file>] [--debug]   build components + kernel
//!   cargo xtask run     [--system <file>] [--debug]   build and boot in QEMU
//!   cargo xtask test                                  host tests + QEMU boot test
//!   cargo xtask qemu-test [--system <file>]           QEMU boot test only
//!   cargo xtask fhd <vendor>/<board>                  validate and show a board
//!   cargo xtask image   --system <file>               flashable boot image for a real board
//!   cargo xtask fbcon-test                            phone-port test double: framebuffer console
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
/// Toolchains for building the Android test apps, pinned by checksum. Apps
/// are built from source (javac/kotlinc → d8/R8 → aapt2); the APKs run on
/// Flare under `droid`, with no Android userspace.
const ANDROID_BUILD_TOOLS: (&str, &str) = (
    "https://dl.google.com/android/repository/build-tools_r35.0.1_linux.zip",
    "5993499f3229a021b89f87088c57242aeefaa62316bf3d69da7de40bfd5350f1",
);
const ANDROID_PLATFORM: (&str, &str) = (
    "https://dl.google.com/android/repository/platform-35_r02.zip",
    "0988cacad01b38a18a47bac14a0695f246bc76c1b06c0eeb8eb0dc825ab0c8e0",
);
const KOTLIN_COMPILER: (&str, &str) = (
    "https://github.com/JetBrains/kotlin/releases/download/v2.1.0/kotlin-compiler-2.1.0.zip",
    "b6698d5728ad8f9edcdd01617d638073191d8a03139cc538a391b4e3759ad297",
);
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
    "pidof",
    "killall",
    "pkill",
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
    "rtc-pl031: wall clock 20",
    "gpio-keys: watching the power key (incarnation 0)",
    "virtio-blk: 64 sectors (32 KiB), incarnation 0, user-space driver with DMA",
    "storage-test: disk of 64 sectors, sector 0 says: FLARE userdata disk",
    "supervisor: restarting 'storage' (incarnation 1)",
    "virtio-blk: 64 sectors (32 KiB), incarnation 1, user-space driver with DMA",
    "storage-test: all reads correct across a driver restart",
    "gpio-keys: power key pressed",
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
    "droid: org.flare.hello: launching org.flare.hello.MainActivity",
    "I/FlareHello: onCreate on Flare virt-phone, API 35",
    "I/FlareHello: sum 1..100 = 5050, fib(20) = 6765, 7/2.0 = 3.5",
    "I/FlareHello: shapes rect,circle total area 15.142",
    "I/FlareHello: word 'the' appears 3 times, 9 distinct words",
    "I/FlareHello: caught IndexOutOfBoundsException",
    "I/FlareHello: lambda ran, created = 1",
    "I/FlareHello: onResume: app is running",
    "droid: org.flare.hello.MainActivity finished cleanly (0 missing APIs)",
    "rc: droid hello.apk exit status 0",
    "I/FlareKotlin: Kotlin onCreate on Flare (API 35)",
    "I/FlareKotlin: data class: Task(id=1, title=boot kernel, done=true), copy: Task(id=1, title=boot kernel, done=false), equal: true",
    "I/FlareKotlin: sealed when: total area 7.14",
    "I/FlareKotlin: groupingBy: {a=3, b=2, c=1}",
    "I/FlareKotlin: range map: [1, 4, 9, 16, 25], sum 55, max 25",
    "I/FlareKotlin: onResume: kotlin app is running",
    "rc: droid kotlin.apk exit status 0",
    "I/SelfTest: selftest: 51/51 passed",
    "rc: droid selftest.apk exit status 0",
    "linux: input device event0: QEMU Virtio Tablet",
    "linux: input device event1: QEMU Virtio Keyboard",
    "virtio-input: 'QEMU Virtio Tablet' online",
    "ramfb: panel 480x854 online (generation 1, incarnation 0)",
    "driver flare: Flare display (shared-memory KMS)",
    "GL renderer softpipe, version OpenGL ES 3.1 Mesa 25.1.9",
    "frame 2 flipped",
    "flarehome: ready on a 480x854 panel, 16 apps, 6 events",
    "flarehome: unlocked",
    "flarehome: page events",
    "flarehome: page applications",
    "flarehome: launched Terminal",
    "flarehome: minimized Terminal (1 open)",
    "flarehome: launched Phone",
    "flarehome: minimized Phone (2 open)",
    "virtio-console: port online (incarnation 0, user-space driver)",
    "art: Hello from ART on Flare: Dalvik 2.1.0",
    "art: words {brown=1, dog=1, end=1, fox=1, jumps=1, lazy=1, over=1, quick=1, the=3}",
    "art: fib(25) = 75025",
    "art: hello from Thread-0",
    "art: 4 processors, pi = 3.14159",
    "rc: art exit status 0",
    "flarehome: android window",
    "flarehome: android window closed",
    "fdbd: serving fdb on /dev/fdb",
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
    "E/SelfTest: FAIL",
    "droid: uncaught exception",
    "droid: missing",
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
    let (comp, manifests) = load_composition(&o.system);
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
    if !names.is_empty() {
        run(c);
    }
    let out = root().join("target").join(TARGET).join(profile);
    let rootfs = assemble_rootfs(comp.linux.as_ref().map(|l| l.rootfs != "minimal").unwrap_or(true));
    if comp.linux.as_ref().is_some_and(|l| !l.hardware.is_empty()) {
        stage_linuxdrv(&comp, &rootfs);
    }
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

/// Download (once) a file and verify its checksum.
fn fetch((url, sha256): (&str, &str)) -> PathBuf {
    let dir = root().join("target").join("downloads");
    let file = dir.join(url.rsplit('/').next().unwrap());
    let ok = file.with_extension("verified");
    if ok.exists() && file.exists() {
        return file;
    }
    std::fs::create_dir_all(&dir).unwrap();
    println!("xtask: downloading {}", url);
    let mut c = Command::new("curl");
    c.args(["-fsSL", "--retry", "3", "-o"]).arg(&file).arg(url);
    run(c);
    let out = Command::new("sha256sum").arg(&file).output().unwrap_or_else(|e| die(&format!("sha256sum: {}", e)));
    let sum = String::from_utf8_lossy(&out.stdout);
    if !sum.starts_with(sha256) {
        let _ = std::fs::remove_file(&file);
        die(&format!("{}: checksum mismatch: {}", url, sum.trim()));
    }
    std::fs::write(&ok, sha256).unwrap();
    file
}

/// Download and unpack (once) a zip; returns the directory it unpacked to.
fn unpacked(dl: (&str, &str)) -> PathBuf {
    let zip = fetch(dl);
    let dir = zip.with_extension("d");
    if !dir.join(".stamp").exists() {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Command::new("unzip");
        c.arg("-q").arg(&zip).arg("-d").arg(&dir);
        run(c);
        std::fs::write(dir.join(".stamp"), "").unwrap();
    }
    dir
}

/// The single top-level directory of an unpacked archive.
fn only_subdir(dir: &Path) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .unwrap_or_else(|| die(&format!("{}: empty archive", dir.display())))
}

fn files_with_ext(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                files_with_ext(&p, ext, out);
            } else if p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }
}

fn newest_mtime(dir: &Path) -> std::time::SystemTime {
    let mut newest = std::time::UNIX_EPOCH;
    for ext in ["java", "kt", "xml"] {
        let mut v = Vec::new();
        files_with_ext(dir, ext, &mut v);
        for f in v {
            if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
                newest = newest.max(m);
            }
        }
    }
    newest
}

/// Build the Android apps in compat/android/apps into APKs (cached until
/// their sources change). Java-only apps are dexed with d8; apps with
/// Kotlin sources are compiled with kotlinc and shrunk with R8 (the Kotlin
/// standard library is part of the app, as in any Android build).
fn build_apks() -> Vec<PathBuf> {
    let apps = root().join("compat/android/apps");
    let out_dir = root().join("target").join("apks");
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(&apps).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    let mut apks = Vec::new();
    for app in dirs {
        let name = app.file_name().unwrap().to_string_lossy().to_string();
        let apk = out_dir.join(format!("{}.apk", name));
        let fresh = apk.metadata().and_then(|m| m.modified()).map(|t| t > newest_mtime(&app)).unwrap_or(false);
        if !fresh {
            build_apk(&app, &apk);
        }
        apks.push(apk);
    }
    apks
}

fn build_apk(app: &Path, apk: &Path) {
    let bt = only_subdir(&unpacked(ANDROID_BUILD_TOOLS));
    let android_jar = only_subdir(&unpacked(ANDROID_PLATFORM)).join("android.jar");
    println!("xtask: building {}", apk.display());
    let work = apk.with_extension("work");
    let _ = std::fs::remove_dir_all(&work);
    let (classes, dex) = (work.join("classes"), work.join("dex"));
    std::fs::create_dir_all(&classes).unwrap();
    std::fs::create_dir_all(&dex).unwrap();
    let (mut java, mut kotlin) = (Vec::new(), Vec::new());
    files_with_ext(&app.join("src"), "java", &mut java);
    files_with_ext(&app.join("src"), "kt", &mut kotlin);
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut cp = android_jar.display().to_string();
    if !kotlin.is_empty() {
        let kc = only_subdir(&unpacked(KOTLIN_COMPILER));
        let stdlib = kc.join("lib/kotlin-stdlib.jar");
        let mut c = Command::new(kc.join("bin/kotlinc"));
        c.args(["-nowarn", "-no-reflect", "-jvm-target", "11", "-cp"]).arg(&android_jar).arg("-d").arg(&classes);
        c.args(&kotlin).args(&java);
        run(c);
        cp = format!("{}:{}:{}", cp, classes.display(), stdlib.display());
        inputs.push(stdlib);
    }
    if !java.is_empty() {
        let mut c = Command::new("javac");
        c.args(["-nowarn", "--release", "11", "-cp", &cp, "-d"]).arg(&classes).args(&java);
        run(c);
    }
    files_with_ext(&classes, "class", &mut inputs);
    let mut c;
    if kotlin.is_empty() {
        c = Command::new(bt.join("d8"));
        c.args(["--min-api", "24"]);
    } else {
        let rules = work.join("keep.pro");
        std::fs::write(
            &rules,
            "-keep public class * extends android.app.Activity { public <init>(); *** on*(...); }\n\
             -dontobfuscate\n-dontwarn **\n",
        )
        .unwrap();
        c = Command::new("java");
        c.arg("-cp").arg(bt.join("lib/d8.jar")).arg("com.android.tools.r8.R8");
        c.args(["--release", "--min-api", "24", "--pg-conf"]).arg(&rules);
    }
    c.arg("--lib").arg(&android_jar).arg("--output").arg(&dex).args(&inputs);
    // R8 warns about Kotlin metadata newer than itself, which is harmless:
    // show its output only if it fails.
    let out = c.output().unwrap_or_else(|e| die(&format!("cannot run {:?}: {}", c, e)));
    if !out.status.success() {
        eprint!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        die(&format!("command failed: {:?}", c));
    }
    let mut c = Command::new(bt.join("aapt2"));
    c.arg("link").arg("-I").arg(&android_jar).arg("--manifest").arg(app.join("AndroidManifest.xml")).arg("-o").arg(apk);
    run(c);
    let mut c = Command::new("zip");
    c.arg("-q").arg(apk).arg("classes.dex").current_dir(&dex);
    run(c);
}

/// Build (once) Arcan 0.6 with Mesa's software rasterizer for Flare's
/// Linux personality; returns the directory to overlay on the rootfs.
fn arcan_stage() -> PathBuf {
    let out = root().join("target").join("arcan");
    // Rebuild when anything the build depends on changes.
    let inputs = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let dir = root().join("compat/arcan");
        let mut files =
            vec![dir.join("build.sh"), dir.join("alpine.lock"), dir.join("alpine.py"), dir.join("stage.py")];
        files.push(dir.join("tests/kmstest.c"));
        let mut patches: Vec<PathBuf> =
            std::fs::read_dir(dir.join("patches")).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        patches.sort();
        files.extend(patches);
        for f in files {
            f.file_name().hash(&mut h);
            std::fs::read(&f).unwrap_or_default().hash(&mut h);
        }
        format!("{:016x}", h.finish())
    };
    let stamp = out.join("stage/.stamp");
    if std::fs::read_to_string(&stamp).map(|s| s.trim() != inputs).unwrap_or(true) {
        println!("xtask: building Arcan and Mesa (once; takes a few minutes)");
        let mut c = Command::new(root().join("compat/arcan/build.sh"));
        c.arg(&out);
        run(c);
        std::fs::write(&stamp, &inputs).unwrap();
    }
    let stage = out.join("stage");
    // The appls in the source tree are copied fresh on every build.
    copy_tree(&root().join("compat/arcan/appl"), &stage.join("usr/share/arcan/appl"));
    stage
}

/// Hash of the files a staged third-party build depends on.
fn inputs_hash(files: &[PathBuf]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in files {
        f.file_name().hash(&mut h);
        std::fs::read(f).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Stage (when its inputs change) ART, the Android runtime, as /opt/art,
/// with a Java check program compiled to DEX.
fn art_stage() -> PathBuf {
    let dir = root().join("compat/android/art");
    let out = root().join("target").join("art");
    let inputs = inputs_hash(&[
        dir.join("build.sh"),
        dir.join("stage.py"),
        dir.join("alpine.lock"),
        dir.join("art-run"),
        root().join("compat/arcan/alpine.py"),
    ]);
    let stamp = out.join("stage/.stamp");
    if std::fs::read_to_string(&stamp).map(|s| s.trim() != inputs).unwrap_or(true) {
        println!("xtask: staging ART (Android runtime)");
        let mut c = Command::new(dir.join("build.sh"));
        c.arg(&out);
        run(c);
        std::fs::write(&stamp, &inputs).unwrap();
    }
    // The check program: javac + d8 into a jar of classes.dex.
    let bt = only_subdir(&unpacked(ANDROID_BUILD_TOOLS));
    let work = out.join("tests");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("classes")).unwrap();
    let mut c = Command::new("javac");
    c.args(["-nowarn", "--release", "11", "-d"]).arg(work.join("classes")).arg(dir.join("tests/HelloArt.java"));
    run(c);
    let mut classes = Vec::new();
    files_with_ext(&work.join("classes"), "class", &mut classes);
    let mut c = Command::new(bt.join("d8"));
    c.args(["--min-api", "26", "--output"]).arg(&work).args(&classes);
    run(c);
    let tests = out.join("stage/opt/art/tests");
    std::fs::create_dir_all(&tests).unwrap();
    let _ = std::fs::remove_file(tests.join("hello-art.jar"));
    let mut c = Command::new("zip");
    c.arg("-q").arg(tests.join("hello-art.jar")).arg("classes.dex").current_dir(&work);
    run(c);
    out.join("stage")
}

/// Build and stage (when its inputs change) Android Translation Layer
/// with Flare's native backend as /opt/atl.
fn atl_stage() -> PathBuf {
    let dir = root().join("compat/android/atl");
    let out = root().join("target").join("atl");
    let mut files = vec![
        dir.join("build.sh"),
        dir.join("stage.py"),
        dir.join("gen_stubs.py"),
        dir.join("alpine.lock"),
        dir.join("atl-run"),
        root().join("compat/arcan/alpine.py"),
    ];
    for sub in ["flare", "patches"] {
        let mut v: Vec<PathBuf> =
            std::fs::read_dir(dir.join(sub)).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        files.extend(v);
    }
    let inputs = inputs_hash(&files);
    let stamp = out.join("stage/.stamp");
    if std::fs::read_to_string(&stamp).map(|s| s.trim() != inputs).unwrap_or(true) {
        println!("xtask: building Android Translation Layer (Flare backend)");
        let bt = only_subdir(&unpacked(ANDROID_BUILD_TOOLS));
        let arcan = arcan_stage();
        let art = art_stage();
        let mut c = Command::new(dir.join("build.sh"));
        c.arg(&out)
            .env("D8", bt.join("d8"))
            .env("ARCAN_ROOT", arcan.parent().unwrap().join("arcan-root"))
            .env("ART_STAGE", &art);
        run(c);
        std::fs::write(&stamp, &inputs).unwrap();
    }
    out.join("stage")
}

/// The Linux driver host (compat/linux/linuxdrv): Linux's own drivers, as a
/// library, in a Linux process. Built when its inputs change.
fn linuxdrv_build() -> PathBuf {
    let dir = root().join("compat/linux/linuxdrv");
    let out = root().join("target/linuxdrv");
    let inputs = inputs_hash(&[
        dir.join("build.sh"),
        dir.join("linuxdrv.c"),
        dir.join("lkl/qrt.patch"),
        dir.join("lkl/flare.patch"),
        dir.join("lkl/config-arm64"),
        dir.join("lkl/config-flare"),
    ]);
    let stamp = out.join(".stamp");
    if std::fs::read_to_string(&stamp).map(|s| s.trim() != inputs).unwrap_or(true) || !out.join("linuxdrv").exists() {
        println!("xtask: building the Linux driver host (LKL; the first time takes about 25 minutes)");
        let sysroot = atl_stage().parent().unwrap().join("sysroot");
        let mut c = Command::new(dir.join("build.sh"));
        c.arg(&out).arg(&sysroot);
        run(c);
        std::fs::write(&stamp, &inputs).unwrap();
    }
    out.join("linuxdrv")
}

/// Put the driver host and its device tree into the root filesystem: the
/// board's own tree (hardware/boards/<board>/linux.dtb, e.g. the mainline
/// one of a phone) or one made from the FHD for the devices given to Linux.
fn stage_linuxdrv(comp: &Composition, rootfs: &Path) {
    let p = flare_fhd::load_board(&root().join("hardware"), &comp.board).unwrap_or_else(|_| exit(1));
    let dir = rootfs.join("etc/linuxdrv");
    std::fs::create_dir_all(&dir).unwrap();
    let board_dir = root().join("hardware/boards").join(&comp.board);
    let own = board_dir.join("linux.dtb");
    let hw = &comp.linux.as_ref().unwrap().hardware;
    if own.exists() {
        // The board's own (mainline) tree, with everything Linux was not
        // given disabled, and the board's notes for Linux (linux.txt).
        let (mut drop, mut cmdline, mut delprop) = (Vec::new(), Vec::new(), Vec::new());
        if let Ok(text) = std::fs::read_to_string(board_dir.join("linux.txt")) {
            for l in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                match l.split_once(' ') {
                    Some(("drop", p)) => drop.push(p.trim().to_string()),
                    Some(("cmdline", a)) => cmdline.push(a.trim().to_string()),
                    Some(("delprop", a)) => delprop.push(a.trim().to_string()),
                    _ => die(&format!("{}/linux.txt: cannot read '{}'", comp.board, l)),
                }
            }
        }
        let mut c = Command::new("python3");
        c.arg("-I").arg(root().join("tools/bootimg/dtb.py")).arg("trim").arg(&own).arg(dir.join("board.dtb"));
        c.arg("--allow");
        for name in hw {
            for r in p.node(name).map(|n| n.regs.clone()).unwrap_or_default() {
                c.arg(format!("{:#x}+{:#x}", r.base, r.size));
            }
        }
        if !drop.is_empty() {
            c.arg("--drop").args(&drop);
        }
        if !delprop.is_empty() {
            c.arg("--delprop").args(&delprop);
        }
        c.stdout(Stdio::null());
        run(c);
        std::fs::write(dir.join("cmdline"), cmdline.join(" ") + "\n").unwrap();
    } else {
        let regs = |n: &flare_fhd::Node| {
            n.regs.iter().map(|r| format!("[{}, {}]", r.base, r.size)).collect::<Vec<_>>().join(", ")
        };
        let strs = |v: &[String]| v.iter().map(|c| format!("{:?}", c)).collect::<Vec<_>>().join(", ");
        let intc = p.enabled_nodes().find(|n| n.class == "intc").unwrap_or_else(|| die("no interrupt controller"));
        let devices: Vec<String> = hw
            .iter()
            .filter_map(|name| p.node(name))
            .map(|n| {
                format!(
                    "{{\"name\": {:?}, \"compatible\": [{}], \"regs\": [{}], \"irqs\": {:?}, \"edge\": {}}}",
                    n.name,
                    strs(&n.compatible),
                    regs(n),
                    n.irqs,
                    n.props.get("linux-irq").and_then(|v| v.as_str()) == Some("edge")
                )
            })
            .collect();
        let spec = format!(
            "{{\"model\": {:?}, \"intc\": {{\"compatible\": [{}], \"regs\": [{}]}}, \"devices\": [{}]}}",
            p.board,
            strs(&intc.compatible),
            regs(intc),
            devices.join(", ")
        );
        let spec_path = root().join("target/linuxdrv-board.json");
        std::fs::write(&spec_path, spec).unwrap();
        let mut c = Command::new("python3");
        c.arg(root().join("tools/bootimg/fhd2dtb.py")).arg(&spec_path).arg(dir.join("board.dtb"));
        run(c);
    }
    std::fs::create_dir_all(rootfs.join("usr/bin")).unwrap();
    std::fs::copy(linuxdrv_build(), rootfs.join("usr/bin/linuxdrv")).unwrap();
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
/// applet links, and the Linux test programs. `full` adds Arcan, ART, ATL
/// and fdbd; boards with a small boot image use the minimal profile.
fn assemble_rootfs(full: bool) -> PathBuf {
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
    if !full {
        symlink("/lib/ld-musl-aarch64.so.1", rootfs.join("lib/libc.musl-aarch64.so.1")).unwrap();
    }
    if full {
        // Arcan, Mesa and their libraries (built by compat/arcan/build.sh).
        let arcan = arcan_stage();
        copy_tree(&arcan, &rootfs);
        symlink("/lib/ld-musl-aarch64.so.1", rootfs.join("lib/libc.musl-aarch64.so.1")).unwrap();
        // ART, the Android runtime (/opt/art, art-run).
        copy_tree(&art_stage(), &rootfs);
        // Android Translation Layer with Flare's native display backend
        // (/opt/atl, atl-run) and its test app.
        copy_tree(&atl_stage(), &rootfs);
        let views = root().join("target/atl/apks/views.apk");
        let fresh = views
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t > newest_mtime(&root().join("compat/android/atl/apps/views")))
            .unwrap_or(false);
        if !fresh {
            std::fs::create_dir_all(views.parent().unwrap()).unwrap();
            build_apk(&root().join("compat/android/atl/apps/views"), &views);
        }
        std::fs::create_dir_all(rootfs.join("data/local/tmp")).unwrap();
        std::fs::copy(&views, rootfs.join("data/local/tmp/views.apk")).unwrap();
        // fdbd, the device end of the debug bridge.
        let mut c = cargo();
        c.args(["build", "--release", "--target", LINUX_TARGET, "-p", "fdbd"]);
        run(c);
        std::fs::copy(root().join("target").join(LINUX_TARGET).join("release/fdbd"), rootfs.join("usr/bin/fdbd"))
            .unwrap();
    }
    // Android apps: the droid runtime and the APKs it runs.
    let mut c = cargo();
    c.args(["build", "--release", "--target", LINUX_TARGET, "-p", "droid"]);
    run(c);
    std::fs::copy(root().join("target").join(LINUX_TARGET).join("release/droid"), rootfs.join("usr/bin/droid"))
        .unwrap();
    let apps = rootfs.join("data/app");
    std::fs::create_dir_all(&apps).unwrap();
    for apk in build_apks() {
        std::fs::copy(&apk, apps.join(apk.file_name().unwrap())).unwrap();
    }
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

/// Boot the phone-port test double (a phone-style memory map and the
/// console on a framebuffer, as on the Xiaomi Mi A1) and read the screen
/// back as text until the Linux personality has run its tests.
fn fbcon_test(o: &Opts) -> bool {
    let mut o2 = Opts { system: "system/qemu-virt-fbcon.boot.toml".into(), release: o.release, rest: Vec::new() };
    if o.system != DEFAULT_SYSTEM {
        o2.system = o.system.clone();
    }
    let (comp, _) = load_composition(&o2.system);
    let p = flare_fhd::load_board(&root().join("hardware"), &comp.board).unwrap_or_else(|_| exit(1));
    let fb = p.nodes.iter().find(|n| n.is_compatible("simple-framebuffer")).unwrap_or_else(|| die("no framebuffer"));
    let int = |k: &str| fb.props.get(k).and_then(|v| v.as_int()).unwrap_or(0);
    let (w, h, stride, scale) = (int("width"), int("height"), int("stride"), int("console-scale").max(1));
    let base = fb.regs[0].base;
    let kernel = build(&o2);
    // Boot it as a phone's boot loader would: the raw Linux Image.
    let image = root().join("target/fbcon-test.Image");
    let mut c = Command::new("llvm-objcopy");
    c.args(["-O", "binary"]).arg(&kernel).arg(&image);
    run(c);
    let qmp = root().join("target/fbcon-qmp.sock");
    let _ = std::fs::remove_file(&qmp);
    // `--load-at ADDR`: load the Image somewhere else and start it there, as
    // a boot loader with its own idea of the address would (the kernel must
    // move itself); no -kernel, so the CPU starts as from reset.
    let load_at = o.rest.iter().position(|a| a == "--load-at").and_then(|i| o.rest.get(i + 1)).cloned();
    let mut cmd = qemu_command(&o2, &image);
    if let Some(addr) = &load_at {
        let args: Vec<_> = cmd.get_args().map(|a| a.to_owned()).collect();
        let mut c = Command::new(cmd.get_program());
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            if a == "-kernel" {
                it.next();
                continue;
            }
            c.arg(a);
        }
        c.arg("-device").arg(format!("loader,file={},addr={},cpu-num=0", image.display(), addr));
        cmd = c;
    }
    let mut child = cmd
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", qmp.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| die(&format!("cannot start qemu-system-aarch64: {}", e)));
    let mut q = None;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(200));
        q = Qmp::connect(&qmp);
        if q.is_some() {
            break;
        }
    }
    let Some(mut q) = q else {
        let _ = child.kill();
        die("no QMP connection")
    };
    let dump = root().join("target/fbcon-test.raw");
    let png = root().join("target/screenshots/fbcon.png");
    let _ = std::fs::create_dir_all(png.parent().unwrap());
    let mut text = String::new();
    let mut seen = String::new(); // every screen decoded: lines scroll off
    let deadline = Instant::now() + Duration::from_secs(300);
    let done = "rc: starting an interactive shell";
    while Instant::now() < deadline && !text.contains(done) && !text.contains("KERNEL PANIC") {
        std::thread::sleep(Duration::from_secs(2));
        q.cmd(&format!(
            "{{\"execute\":\"pmemsave\",\"arguments\":{{\"val\":{},\"size\":{},\"filename\":\"{}\"}}}}",
            base,
            stride * h,
            dump.display()
        ));
        let out = Command::new("python3")
            .arg(root().join("tools/bootimg/fbtext.py"))
            .arg(&dump)
            .args([w, h, stride, 3, scale].iter().map(|v| v.to_string()))
            .arg(&png)
            .output()
            .unwrap_or_else(|e| die(&format!("fbtext.py: {}", e)));
        text = String::from_utf8_lossy(&out.stdout).into_owned();
        seen.push_str(&text);
    }
    let _ = child.kill();
    let _ = child.wait();
    println!("---- screen ({}) ----\n{}", png.display(), text);
    let flat = seen.replace('\n', "");
    let mut ok = text.contains(done) && !seen.contains("KERNEL PANIC");
    for want in ["rc: input devices: event0", "keys waiting"] {
        let found = flat.split_whitespace().collect::<Vec<_>>().join(" ").contains(want);
        println!("{}: {}", want, if found { "seen" } else { "MISSING" });
        ok &= found;
    }
    for bad in ["' faulted:", "kernel: restarting", "cannot start"] {
        if flat.contains(bad) {
            println!("unexpected: {}", bad);
            ok = false;
        }
    }
    println!("{}", if ok { "PASS: the console reached the shell on the framebuffer" } else { "FAIL" });
    ok
}

/// Build the composition and package the kernel the way the board's boot
/// loader wants it (`[boot-image]` in its FHD), in target/images/<composition name>.
fn image(o: &Opts) {
    let (comp, _) = load_composition(&o.system);
    let p = flare_fhd::load_board(&root().join("hardware"), &comp.board).unwrap_or_else(|errs| {
        for e in errs {
            eprintln!("{}", e);
        }
        exit(1)
    });
    if p.boot_image.is_empty() {
        die(&format!("board {} has no [boot-image] section (a virtual board? use `cargo xtask run`)", comp.board));
    }
    let kernel = build(o);
    let out = root().join("target/images").join(&comp.name);
    std::fs::create_dir_all(&out).unwrap();
    let image = out.join("Image");
    let mut c = Command::new("llvm-objcopy");
    c.args(["-O", "binary"]).arg(&kernel).arg(&image);
    run(c);
    fn json(v: &flare_config::Value) -> String {
        match v {
            flare_config::Value::Str(s) => format!("{:?}", s),
            flare_config::Value::Int(i) => i.to_string(),
            flare_config::Value::Bool(b) => b.to_string(),
            flare_config::Value::Array(a) => format!("[{}]", a.iter().map(json).collect::<Vec<_>>().join(", ")),
        }
    }
    let spec = format!(
        "{{{}}}",
        p.boot_image.iter().map(|(k, v)| format!("{:?}: {}", k, json(v))).collect::<Vec<_>>().join(", ")
    );
    let spec_path = out.join("boot-image.json");
    std::fs::write(&spec_path, spec).unwrap();
    let mut c = Command::new("python3");
    c.arg(root().join("tools/bootimg/mkimage.py")).arg(&spec_path).arg(&image).arg(&out);
    run(c);
    println!("image: {}", out.join("boot.img").display());
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
    // Emulated devices the board describes; @disk is the userdata image.
    let disk = disk_image();
    if let Some(args) = p.emulation.get("qemu-args").and_then(|v| v.as_str_list()) {
        for a in args {
            c.arg(a.replace("@disk", &disk.display().to_string()).replace("@fdb", &fdb_socket().display().to_string()));
        }
    }
    c
}

/// The host end of the debug bridge (the board's virtio console).
fn fdb_socket() -> PathBuf {
    let p = root().join("target/fdb.sock");
    let _ = std::fs::remove_file(&p);
    p
}

/// The userdata disk: sector 0 names it, sector n starts "FLARE-SECTOR-n".
fn disk_image() -> PathBuf {
    let path = root().join("target/flare-disk.img");
    let mut img = vec![0u8; 64 * 512];
    let head = b"FLARE userdata disk\n";
    img[..head.len()].copy_from_slice(head);
    for n in 1..64 {
        let tag = format!("FLARE-SECTOR-{:04}", n);
        img[n * 512..n * 512 + tag.len()].copy_from_slice(tag.as_bytes());
    }
    std::fs::write(&path, img).unwrap_or_else(|e| die(&format!("cannot write {}: {}", path.display(), e)));
    path
}

/// A QEMU QMP connection (JSON lines over a Unix socket).
struct Qmp {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
    w: std::os::unix::net::UnixStream,
}

impl Qmp {
    fn connect(sock: &Path) -> Option<Qmp> {
        use std::io::BufRead;
        let stream = std::os::unix::net::UnixStream::connect(sock).ok()?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut q = Qmp { reader: std::io::BufReader::new(stream.try_clone().ok()?), w: stream };
        let mut line = String::new();
        q.reader.read_line(&mut line).ok()?; // greeting
        q.cmd("{\"execute\":\"qmp_capabilities\"}")?;
        Some(q)
    }

    /// Run a command; its reply (events in between are skipped).
    fn cmd(&mut self, json: &str) -> Option<String> {
        use std::io::BufRead;
        self.w.write_all(json.as_bytes()).ok()?;
        self.w.write_all(b"\n").ok()?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line).ok()? == 0 {
                return None;
            }
            if line.contains("\"return\"") || line.contains("\"error\"") {
                return Some(line);
            }
        }
    }

    /// Move the virtio tablet to panel coordinates (480x854).
    fn point(&mut self, x: f64, y: f64) {
        let (ax, ay) = ((x * 32767.0 / 480.0) as i64, (y * 32767.0 / 854.0) as i64);
        self.cmd(&format!(
            "{{\"execute\":\"input-send-event\",\"arguments\":{{\"events\":[{{\"type\":\"abs\",\"data\":{{\"axis\":\"x\",\"value\":{}}}}},{{\"type\":\"abs\",\"data\":{{\"axis\":\"y\",\"value\":{}}}}}]}}}}",
            ax, ay
        ));
    }

    fn button(&mut self, down: bool) {
        self.cmd(&format!(
            "{{\"execute\":\"input-send-event\",\"arguments\":{{\"events\":[{{\"type\":\"btn\",\"data\":{{\"down\":{},\"button\":\"left\"}}}}]}}}}",
            down
        ));
    }

    fn tap(&mut self, x: f64, y: f64) {
        self.point(x, y);
        std::thread::sleep(Duration::from_millis(100));
        self.button(true);
        std::thread::sleep(Duration::from_millis(150));
        self.button(false);
    }

    fn swipe(&mut self, from: (f64, f64), to: (f64, f64)) {
        self.point(from.0, from.1);
        std::thread::sleep(Duration::from_millis(100));
        self.button(true);
        for i in 1..=8 {
            std::thread::sleep(Duration::from_millis(60));
            let t = i as f64 / 8.0;
            self.point(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        }
        std::thread::sleep(Duration::from_millis(100));
        self.button(false);
    }

    /// Save the display once it has stopped changing (frames are slow on
    /// a software rasterizer under emulation).
    fn screenshot(&mut self, path: &Path) {
        let tmp = path.with_extension("tmp.png");
        let grab = |q: &mut Qmp| -> Vec<u8> {
            q.cmd(&format!(
                "{{\"execute\":\"screendump\",\"arguments\":{{\"filename\":\"{}\",\"format\":\"png\"}}}}",
                tmp.display()
            ));
            std::fs::read(&tmp).unwrap_or_default()
        };
        std::thread::sleep(Duration::from_secs(3));
        let mut prev = grab(self);
        for _ in 0..12 {
            std::thread::sleep(Duration::from_secs(3));
            let cur = grab(self);
            if cur == prev {
                break;
            }
            prev = cur;
        }
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Wait until the console output contains `text`.
fn wait_output(output: &Mutex<Vec<u8>>, text: &str, secs: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if String::from_utf8_lossy(&output.lock().unwrap()).contains(text) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Boot a composition that gives hardware to Linux and check that Linux's
/// own drivers, running in the driver host, reached it.
fn linuxdrv_test(o: &Opts) -> bool {
    let mut o2 = Opts { system: "system/qemu-virt-linuxdrv.boot.toml".into(), release: o.release, rest: Vec::new() };
    if o.system != DEFAULT_SYSTEM {
        o2.system = o.system.clone();
    }
    let kernel = build(&o2);
    let qmp = root().join("target/linuxdrv-qmp.sock");
    let _ = std::fs::remove_file(&qmp);
    let mut child = qemu_command(&o2, &kernel)
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", qmp.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| die(&format!("cannot start qemu-system-aarch64: {}", e)));
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut stdout = child.stdout.take().unwrap();
    let sink = output.clone();
    std::thread::spawn(move || {
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
    let expected = [
        "linuxdrv: Linux is up",
        "linuxdrv: virtio31: 0xa003e00+0x200 irq 79",
        "linuxdrv: block devices: vda",
        // The userdata disk starts with "FLAR".
        "linuxdrv: vda: first sector read through Linux's driver: 46 4c 41 52",
        // Several requests, each completed by an interrupt (edge-triggered
        // here: board.fhd), as a phone's RPM replies are.
        "linuxdrv: vda: 8 of 8 uncached reads",
        "linuxdrv: rtc0 (Linux's driver): 20",
        // A key pressed on QEMU's virtio keyboard, through Linux's driver
        // (sent once the report is out): KEY_A.
        "key 30 pressed",
    ];
    let fatal = ["KERNEL PANIC", "' faulted:", "kernel: restarting", "linuxdrv: Linux did not start", "linuxdrv: Linux panicked"];
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut pressed = false;
    let ok = loop {
        let seen = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
        if !pressed && seen.contains("linuxdrv: input devices:") {
            std::thread::sleep(Duration::from_secs(1));
            if let Some(mut q) = Qmp::connect(&qmp) {
                q.cmd(r#"{"execute":"send-key","arguments":{"keys":[{"type":"qcode","data":"a"}]}}"#);
            }
            pressed = true;
        }
        if expected.iter().all(|e| seen.contains(e)) {
            break true;
        }
        if fatal.iter().any(|f| seen.contains(f)) || Instant::now() > deadline || child.try_wait().unwrap().is_some() {
            break false;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let _ = child.kill();
    let _ = child.wait();
    let seen = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    println!();
    for e in expected {
        println!("linuxdrv-test: {} {:?}", if seen.contains(e) { "ok     " } else { "MISSING" }, e);
    }
    println!("linuxdrv-test: {}", if ok { "PASS" } else { "FAIL" });
    ok
}

/// Drive the phone shell through the touchscreen, as a user would, and
/// keep screenshots in target/screenshots. The shell logs each step; the
/// expected lines check them.
fn ui_script(qmp: &mut Qmp, output: &Mutex<Vec<u8>>) {
    let dir = root().join("target/screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let shot = |q: &mut Qmp, name: &str| q.screenshot(&dir.join(format!("{}.png", name)));
    let (left, right) = ((60.0, 500.0), (420.0, 500.0));
    shot(qmp, "1-lock");
    qmp.swipe((60.0, 600.0), (400.0, 600.0));
    wait_output(output, "flarehome: unlocked", 30);
    shot(qmp, "2-applications");
    qmp.swipe(left, right);
    wait_output(output, "flarehome: page events", 30);
    shot(qmp, "3-events");
    qmp.swipe(right, left);
    wait_output(output, "flarehome: page applications", 30);
    shot(qmp, "tmp");
    // Terminal: third row, third column of the launcher
    qmp.tap(300.0, 356.0);
    wait_output(output, "flarehome: launched Terminal", 30);
    shot(qmp, "4-terminal");
    qmp.swipe((470.0, 400.0), (150.0, 400.0));
    wait_output(output, "flarehome: minimized Terminal", 30);
    shot(qmp, "tmp");
    qmp.swipe(left, right);
    shot(qmp, "tmp");
    // Phone: first row, first column
    qmp.tap(60.0, 100.0);
    wait_output(output, "flarehome: launched Phone", 30);
    shot(qmp, "tmp");
    qmp.swipe((5.0, 400.0), (330.0, 400.0));
    wait_output(output, "flarehome: minimized Phone (2 open)", 30);
    shot(qmp, "5-open-applications");
    let _ = std::fs::remove_file(dir.join("tmp.png"));
}

/// Exercise the debug bridge from the host, as a developer would; returns
/// what failed.
fn fdb_checks() -> Vec<String> {
    let mut fails = Vec::new();
    let sock = root().join("target/fdb.sock");
    let stream = match std::os::unix::net::UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(e) => return vec![format!("fdb: cannot connect to {}: {}", sock.display(), e)],
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut c = fdb::Client::new(stream);
    let mut check = |what: &str, ok: bool| {
        println!("xtask: fdb {}: {}", what, if ok { "ok" } else { "FAILED" });
        if !ok {
            fails.push(format!("fdb {}", what));
        }
    };
    let hello = c.hello().unwrap_or_default();
    check("hello", hello.contains("flare device"));
    let out = c.exec("echo fdb-$((6 * 7))").ok();
    check("shell", out.as_ref().is_some_and(|o| o.stdout == b"fdb-42\n" && o.status == 0));
    check("exit status", c.exec("exit 3").map(|o| o.status == 3).unwrap_or(false));
    let data: Vec<u8> = (0..200_000u32).map(|i| (i * 7 + i / 251) as u8).collect();
    let round = c.write_file("/tmp/fdb-check/blob", 0o600, &data).and_then(|_| c.read_file("/tmp/fdb-check/blob"));
    check("push and pull", round.map(|d| d == data).unwrap_or(false));
    let shot = c.screenshot();
    check(
        "screenshot",
        shot.as_ref().map(|(w, h, px)| (*w, *h, px.len()) == (480, 854, 480 * 854 * 3)).unwrap_or(false),
    );
    if let Ok((w, h, px)) = shot {
        let _ = std::fs::write(root().join("target/screenshots/fdb.png"), fdb::png(w, h, &px));
    }
    check(
        "supervisor",
        c.exec("echo list > /dev/flarectl && cat /dev/flarectl")
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("debug-bridge"))
            .unwrap_or(false),
    );
    fails
}

/// Run an Android app on ART and ATL in the phone shell, as a user would:
/// start it over fdb, wait for its window, tap a button and check that
/// the app's click listener ran; returns what failed.
fn android_app_check(qmp: &mut Qmp, output: &Mutex<Vec<u8>>) -> Vec<String> {
    let mut fails = Vec::new();
    let sock = root().join("target/fdb.sock");
    let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock) else {
        return vec!["android: no fdb connection".into()];
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut c = fdb::Client::new(stream);
    let mut check = |what: &str, ok: bool| {
        println!("xtask: android {}: {}", what, if ok { "ok" } else { "FAILED" });
        if !ok {
            fails.push(format!("android {}", what));
        }
    };
    let log_has = |c: &mut fdb::Client<std::os::unix::net::UnixStream>, text: &str, secs: u64| {
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            if c.exec(&format!("grep -q '{}' /tmp/atl.log", text)).map(|o| o.status == 0).unwrap_or(false) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        false
    };
    let started = c.exec("atl-run /data/local/tmp/views.apk > /tmp/atl.log 2>&1 &").map(|o| o.status == 0);
    check("launch", started.unwrap_or(false));
    check("window", wait_output(output, "flarehome: android window", 120));
    check("first frame", log_has(&mut c, "atl: first frame drawn", 120));
    std::thread::sleep(Duration::from_secs(3)); // the zoom-in animation
    let dir = root().join("target/screenshots");
    qmp.screenshot(&dir.join("6-android-app.png"));
    // the app's "Turn" button: tapped through the shell, which forwards
    // the touch to the app's window
    qmp.tap(132.0, 452.0);
    check("click", log_has(&mut c, "FlareViews.*click 1", 60));
    std::thread::sleep(Duration::from_secs(3));
    qmp.screenshot(&dir.join("7-android-app-clicked.png"));
    let _ = c.exec("killall ld-musl-aarch64.so.1");
    check("close", wait_output(output, "flarehome: android window closed", 60));
    fails
}

/// Press the board's power key through QEMU's QMP socket.
fn qmp_powerdown(sock: &Path) -> bool {
    use std::io::{BufRead, BufReader};
    let Ok(stream) = std::os::unix::net::UnixStream::connect(sock) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut w = stream;
    let mut line = String::new();
    let _ = reader.read_line(&mut line); // greeting
    for cmd in ["{\"execute\":\"qmp_capabilities\"}\n", "{\"execute\":\"system_powerdown\"}\n"] {
        if w.write_all(cmd.as_bytes()).is_err() {
            return false;
        }
        line.clear();
        let _ = reader.read_line(&mut line);
    }
    true
}

fn qemu_test(o: &Opts) -> bool {
    let kernel = build(o);
    println!("xtask: booting {} in QEMU", kernel.display());
    let qmp = root().join("target/qemu-qmp.sock");
    let _ = std::fs::remove_file(&qmp);
    let mut child = qemu_command(o, &kernel)
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", qmp.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| die(&format!("cannot start qemu-system-aarch64: {}", e)));
    let mut stdin = child.stdin.take().unwrap();
    let mut typed = false;
    let mut pressed = false;
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
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut timed_out = false;
    let mut ui_done = false;
    let mut fdb_fails = Vec::new();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        // Once the interactive shell is up: press the power key, and when
        // the keys driver has reported it, type into the shell.
        let seen = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
        // First the phone shell, through the touchscreen.
        if !ui_done && seen.contains("flarehome: ready") {
            if let Some(mut q) = Qmp::connect(&qmp) {
                ui_script(&mut q, &output);
                fdb_fails = fdb_checks();
                fdb_fails.extend(android_app_check(&mut q, &output));
            } else {
                fdb_fails = fdb_checks();
            }
            ui_done = true;
        }
        if !pressed && ui_done && seen.contains("flare:~# ") {
            pressed = qmp_powerdown(&qmp);
        }
        if !typed && pressed && seen.contains("gpio-keys: power key pressed") {
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
        println!("FAIL: QEMU did not halt within 600 s");
    }
    for e in EXPECTED {
        if !text.contains(e) {
            println!("FAIL: missing output: {}", e);
            ok = false;
        }
    }
    for f in &fdb_fails {
        println!("FAIL: {}", f);
        ok = false;
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
    let (comp, all) = load_composition(&o.system);
    let mut manifests: Vec<Manifest> = Vec::new();
    for m in all {
        if !manifests.iter().any(|u| u.name == m.name) {
            manifests.push(m);
        }
    }
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
        eprintln!("usage: cargo xtask <build|run|test|qemu-test|image|fhd|cm> [options]");
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
        "image" => image(&o),
        "fbcon-test" => exit(if fbcon_test(&o) { 0 } else { 1 }),
        "linuxdrv-test" => exit(if linuxdrv_test(&o) { 0 } else { 1 }),
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
