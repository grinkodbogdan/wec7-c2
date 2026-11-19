//! fdbd: the device end of fdb, the Flare Debug Bridge.
//!
//! Serves the fdb protocol (see the `fdb` crate) on /dev/fdb, a byte stream
//! that a user-space virtio-console driver carries to the host. Commands
//! run under /bin/sh with their output streamed back as it is produced.

use fdb::{kind, read_frame, write_frame};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The device's write side: /dev/fdb pushes back (EAGAIN) when the driver
/// has not drained it yet, so writes wait and retry.
struct Out(File);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        loop {
            match self.0.write(b) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
                r => return r,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

type Shared = Arc<Mutex<Out>>;

fn send(out: &Shared, k: u8, payload: &[u8]) {
    let _ = write_frame(&mut *out.lock().unwrap(), k, payload);
}

fn ok_or_fail(out: &Shared, r: io::Result<()>) {
    match r {
        Ok(()) => send(out, kind::OK, &[]),
        Err(e) => send(out, kind::FAIL, e.to_string().as_bytes()),
    }
}

fn hello() -> String {
    let uname = fs::read_to_string("/proc/version").unwrap_or_default();
    let uptime = fs::read_to_string("/proc/uptime").unwrap_or_default();
    let secs = uptime.split_whitespace().next().unwrap_or("?");
    format!("flare device (fdbd {}), up {} s\n{}", env!("CARGO_PKG_VERSION"), secs, uname.trim())
}

/// Run `sh -c cmd`, streaming its output; returns its stdin.
fn exec(out: &Shared, cmd: &str) -> Option<ChildStdin> {
    let mut child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            send(out, kind::FAIL, e.to_string().as_bytes());
            return None;
        }
    };
    let pump = |mut r: Box<dyn Read + Send>, k: u8, out: Shared| {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = r.read(&mut buf) {
                if n == 0 {
                    break;
                }
                send(&out, k, &buf[..n]);
            }
        })
    };
    let a = pump(Box::new(child.stdout.take().unwrap()), kind::STDOUT, out.clone());
    let b = pump(Box::new(child.stderr.take().unwrap()), kind::STDERR, out.clone());
    let stdin = child.stdin.take();
    let out = out.clone();
    std::thread::spawn(move || {
        let _ = a.join();
        let _ = b.join();
        let status = child.wait().map(|s| s.code().unwrap_or(128)).unwrap_or(127);
        send(&out, kind::EXIT, &status.to_le_bytes());
    });
    stdin
}

extern "C" {
    fn ioctl(fd: i32, req: u64, ...) -> i32;
}

/// The display through /dev/fb0: width, height, stride, then the pixels.
fn screenshot() -> io::Result<Vec<u8>> {
    let mut fb = File::open("/dev/fb0")?;
    let (mut var, mut fix) = ([0u8; 160], [0u8; 80]);
    // SAFETY: the buffers have the size of fb_var_screeninfo / fb_fix_screeninfo.
    unsafe {
        if ioctl(fb.as_raw_fd(), 0x4600, var.as_mut_ptr()) != 0 || ioctl(fb.as_raw_fd(), 0x4602, fix.as_mut_ptr()) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let rd = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let (w, h, stride) = (rd(&var, 0), rd(&var, 4), rd(&fix, 48));
    let mut out = Vec::with_capacity(12 + (stride * h) as usize);
    for v in [w, h, stride] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut px = vec![0u8; (stride * h) as usize];
    fb.read_exact(&mut px)?;
    out.extend_from_slice(&px);
    Ok(out)
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/dev/fdb".into());
    let mut input = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("fdbd: {}: {}", path, e);
            std::process::exit(1);
        }
    };
    let out: Shared = Arc::new(Mutex::new(Out(input.try_clone().expect("dup /dev/fdb"))));
    println!("fdbd: serving fdb on {}", path);
    let mut stdin: Option<ChildStdin> = None;
    loop {
        let (k, p) = match read_frame(&mut input) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("fdbd: {}", e);
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        match k {
            kind::HELLO => send(&out, kind::HELLO, hello().as_bytes()),
            kind::EXEC => stdin = exec(&out, &String::from_utf8_lossy(&p)),
            kind::STDIN => {
                if let Some(s) = stdin.as_mut() {
                    let _ = s.write_all(&p);
                }
            }
            kind::CLOSE_STDIN => stdin = None,
            kind::WRITE => {
                let r = match fdb::parse_write(&p) {
                    Some((mode, path, data)) => (|| {
                        if let Some(dir) = std::path::Path::new(&path).parent() {
                            fs::create_dir_all(dir)?;
                        }
                        fs::write(&path, data)?;
                        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                    })(),
                    None => Err(io::Error::new(io::ErrorKind::InvalidData, "bad write request")),
                };
                ok_or_fail(&out, r);
            }
            kind::READ => match fs::read(String::from_utf8_lossy(&p).as_ref()) {
                Ok(d) => send(&out, kind::DATA, &d),
                Err(e) => send(&out, kind::FAIL, e.to_string().as_bytes()),
            },
            kind::MKDIR if p.len() >= 4 => {
                let mode = u32::from_le_bytes(p[..4].try_into().unwrap());
                let path = String::from_utf8_lossy(&p[4..]).into_owned();
                let r = fs::create_dir_all(&path)
                    .and_then(|_| fs::set_permissions(&path, fs::Permissions::from_mode(mode)));
                ok_or_fail(&out, r);
            }
            kind::SCREENSHOT => match screenshot() {
                Ok(d) => send(&out, kind::DATA, &d),
                Err(e) => send(&out, kind::FAIL, e.to_string().as_bytes()),
            },
            other => send(&out, kind::FAIL, format!("unknown request {:?}", other as char).as_bytes()),
        }
    }
}
