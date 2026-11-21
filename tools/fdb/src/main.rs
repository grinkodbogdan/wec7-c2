//! fdb: talk to a Flare device, like adb does for Android.
//!
//! ```text
//! fdb devices                     who is connected
//! fdb shell [command...]          run a command (interactive sh without one)
//! fdb push <local> <remote>       copy a file or directory to the device
//! fdb pull <remote> [local]       copy a file from the device
//! fdb install <app.apk>           copy an APK to /data/app
//! fdb run <app.apk>               install it and run it with droid
//! fdb appl <dir> [name]           push an Arcan appl and restart the UI with it
//! fdb ui [appl]                   restart the UI
//! fdb log                         follow the UI's log
//! fdb ctl <command...>            ask the supervisor (list, restart <instance>, ...)
//! fdb screenshot [out.png]        save the display
//! ```
//!
//! The connection is `$FDB_SOCKET` (a Unix socket path, `tcp:host:port`, or
//! a serial device such as `/dev/ttyACM0`: a phone over USB), by default
//! the socket `cargo xtask run` gives QEMU: target/fdb.sock.

use fdb::{kind, read_frame, write_frame, Client};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

/// A Unix, TCP or serial (USB ACM) connection.
enum Conn {
    Unix(std::os::unix::net::UnixStream),
    Tcp(std::net::TcpStream),
    Serial(std::fs::File),
}

impl Conn {
    fn open() -> io::Result<Conn> {
        let spec = std::env::var("FDB_SOCKET").unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fdb.sock").display().to_string()
        });
        // A serial device (a phone's USB serial gadget): raw, 8 bits.
        let is_serial = std::fs::metadata(&spec)
            .map(|m| {
                use std::os::unix::fs::FileTypeExt;
                m.file_type().is_char_device()
            })
            .unwrap_or(false);
        if is_serial {
            let ok = std::process::Command::new("stty")
                .args(["-F", &spec, "raw", "-echo", "-echoe", "-echok", "-icrnl", "-onlcr", "-ixon", "cs8"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err(io::Error::other(format!("{}: cannot set raw mode (stty)", spec)));
            }
            let f = std::fs::OpenOptions::new().read(true).write(true).open(&spec)?;
            return Ok(Conn::Serial(f));
        }
        match spec.strip_prefix("tcp:") {
            Some(addr) => Ok(Conn::Tcp(std::net::TcpStream::connect(addr)?)),
            None => std::os::unix::net::UnixStream::connect(&spec)
                .map(Conn::Unix)
                .map_err(|e| io::Error::new(e.kind(), format!("{}: {} (is the device running?)", spec, e))),
        }
    }
    fn try_clone(&self) -> io::Result<Conn> {
        Ok(match self {
            Conn::Unix(s) => Conn::Unix(s.try_clone()?),
            Conn::Tcp(s) => Conn::Tcp(s.try_clone()?),
            Conn::Serial(f) => Conn::Serial(f.try_clone()?),
        })
    }
}

impl Read for Conn {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Unix(s) => s.read(b),
            Conn::Tcp(s) => s.read(b),
            Conn::Serial(f) => f.read(b),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Unix(s) => s.write(b),
            Conn::Tcp(s) => s.write(b),
            Conn::Serial(f) => f.write(b),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Unix(s) => s.flush(),
            Conn::Tcp(s) => s.flush(),
            Conn::Serial(f) => f.flush(),
        }
    }
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("fdb: {}", msg);
    exit(1)
}

fn connect() -> Client<Conn> {
    let conn = Conn::open().unwrap_or_else(|e| die(e));
    let mut c = Client::new(conn);
    c.hello().unwrap_or_else(|e| die(format!("no answer from the device: {}", e)));
    c
}

/// Single-quote for sh.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn run(c: &mut Client<Conn>, cmd: &str) -> i32 {
    c.exec_stream(cmd, |err, b| {
        if err {
            let _ = io::stderr().write_all(b);
        } else {
            let _ = io::stdout().write_all(b);
            let _ = io::stdout().flush();
        }
    })
    .unwrap_or_else(|e| die(e))
}

fn interactive(c: Client<Conn>) -> i32 {
    let mut c = c;
    let mut tx = c.stream().try_clone().unwrap_or_else(|e| die(e));
    write_frame(c.stream(), kind::EXEC, b"PS1='flare$ ' exec /bin/sh -i 2>&1").unwrap_or_else(|e| die(e));
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match io::stdin().read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = write_frame(&mut tx, kind::CLOSE_STDIN, &[]);
                    return;
                }
                Ok(n) => {
                    if write_frame(&mut tx, kind::STDIN, &buf[..n]).is_err() {
                        return;
                    }
                }
            }
        }
    });
    loop {
        match read_frame(c.stream()) {
            Ok((kind::STDOUT, b)) | Ok((kind::STDERR, b)) => {
                let _ = io::stdout().write_all(&b);
                let _ = io::stdout().flush();
            }
            Ok((kind::EXIT, b)) => return i32::from_le_bytes(b[..4].try_into().unwrap_or([0; 4])),
            Ok((kind::FAIL, b)) => die(String::from_utf8_lossy(&b)),
            Ok(_) => {}
            Err(e) => die(e),
        }
    }
}

fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644)
}

/// Copy a file or a directory tree; returns (files, bytes).
fn push(c: &mut Client<Conn>, local: &Path, remote: &str) -> (usize, usize) {
    if local.is_dir() {
        c.mkdir(remote, 0o755).unwrap_or_else(|e| die(format!("{}: {}", remote, e)));
        let mut entries: Vec<PathBuf> = std::fs::read_dir(local)
            .unwrap_or_else(|e| die(format!("{}: {}", local.display(), e)))
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        let (mut files, mut bytes) = (0, 0);
        for e in entries {
            let name = e.file_name().unwrap().to_string_lossy().to_string();
            let (f, b) = push(c, &e, &format!("{}/{}", remote.trim_end_matches('/'), name));
            files += f;
            bytes += b;
        }
        (files, bytes)
    } else {
        let data = std::fs::read(local).unwrap_or_else(|e| die(format!("{}: {}", local.display(), e)));
        c.write_file(remote, mode_of(local), &data).unwrap_or_else(|e| die(format!("{}: {}", remote, e)));
        (1, data.len())
    }
}

/// `push` to a directory target keeps the source's name, like cp.
fn push_target(c: &mut Client<Conn>, local: &Path, remote: &str) -> String {
    let is_dir = c.exec(&format!("test -d {}", quote(remote))).map(|o| o.status == 0).unwrap_or(false);
    if is_dir || remote.ends_with('/') {
        format!("{}/{}", remote.trim_end_matches('/'), local.file_name().unwrap().to_string_lossy())
    } else {
        remote.to_string()
    }
}

fn install(c: &mut Client<Conn>, apk: &Path) -> String {
    let name = apk.file_name().unwrap_or_else(|| die("no file name")).to_string_lossy().to_string();
    let remote = format!("/data/app/{}", name);
    let data = std::fs::read(apk).unwrap_or_else(|e| die(format!("{}: {}", apk.display(), e)));
    c.mkdir("/data/app", 0o755).unwrap_or_else(|e| die(e));
    c.write_file(&remote, 0o644, &data).unwrap_or_else(|e| die(e));
    println!("fdb: installed {} ({} KiB)", remote, data.len().div_ceil(1024));
    remote
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let code = match cmd {
        "devices" | "hello" => {
            let mut c = connect();
            let info = c.hello().unwrap_or_else(|e| die(e));
            println!("{}", info.trim_end());
            0
        }
        "shell" if rest.is_empty() => interactive(connect()),
        "shell" => run(&mut connect(), &rest.join(" ")),
        "push" if rest.len() == 2 => {
            let mut c = connect();
            let local = Path::new(&rest[0]);
            let remote = push_target(&mut c, local, &rest[1]);
            let t = std::time::Instant::now();
            let (files, bytes) = push(&mut c, local, &remote);
            println!(
                "fdb: {} -> {}: {} file(s), {} KiB in {:.1} s",
                local.display(),
                remote,
                files,
                bytes.div_ceil(1024),
                t.elapsed().as_secs_f64()
            );
            0
        }
        "pull" if !rest.is_empty() => {
            let mut c = connect();
            let data = c.read_file(&rest[0]).unwrap_or_else(|e| die(format!("{}: {}", rest[0], e)));
            let local = rest.get(1).cloned().unwrap_or_else(|| rest[0].rsplit('/').next().unwrap().to_string());
            std::fs::write(&local, &data).unwrap_or_else(|e| die(format!("{}: {}", local, e)));
            println!("fdb: {} -> {}: {} KiB", rest[0], local, data.len().div_ceil(1024));
            0
        }
        "install" if rest.len() == 1 => {
            install(&mut connect(), Path::new(&rest[0]));
            0
        }
        "run" if rest.len() == 1 => {
            let mut c = connect();
            let remote = install(&mut c, Path::new(&rest[0]));
            run(&mut c, &format!("/usr/bin/droid {}", quote(&remote)))
        }
        "appl" if !rest.is_empty() => {
            let mut c = connect();
            let dir = Path::new(&rest[0]);
            let name = rest.get(1).cloned().unwrap_or_else(|| {
                dir.canonicalize().unwrap_or(dir.into()).file_name().unwrap().to_string_lossy().to_string()
            });
            let remote = format!("/usr/share/arcan/appl/{}", name);
            let t = std::time::Instant::now();
            let (files, _) = push(&mut c, dir, &remote);
            let status = run(&mut c, &format!("/usr/bin/flare-ui {}", quote(&name)));
            println!("fdb: appl '{}' ({} files) restarted in {:.1} s", name, files, t.elapsed().as_secs_f64());
            status
        }
        "ui" => {
            let appl = rest.first().map(String::as_str).unwrap_or("flarehome");
            run(&mut connect(), &format!("/usr/bin/flare-ui {}", quote(appl)))
        }
        "log" => run(&mut connect(), "tail -n 50 -f /tmp/flare-ui.log"),
        "ctl" if !rest.is_empty() => {
            run(&mut connect(), &format!("echo {} > /dev/flarectl && cat /dev/flarectl", quote(&rest.join(" "))))
        }
        "screenshot" => {
            let mut c = connect();
            let (w, h, rgb) = c.screenshot().unwrap_or_else(|e| die(e));
            let out = rest.first().cloned().unwrap_or_else(|| "screenshot.png".into());
            std::fs::write(&out, fdb::png(w, h, &rgb)).unwrap_or_else(|e| die(format!("{}: {}", out, e)));
            println!("fdb: {}x{} display -> {}", w, h, out);
            0
        }
        _ => {
            eprintln!(
                "usage: fdb <command>\n\n  devices | shell [cmd...] | push <local> <remote> | pull <remote> [local]\n  install <apk> | run <apk> | appl <dir> [name] | ui [appl] | log\n  ctl <supervisor command> | screenshot [out.png]\n\nconnection: $FDB_SOCKET (unix path, tcp:host:port, or a serial device: /dev/ttyACM0 for a phone over USB), default target/fdb.sock"
            );
            2
        }
    };
    exit(code)
}
