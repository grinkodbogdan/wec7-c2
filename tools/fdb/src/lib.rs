//! fdb, the Flare Debug Bridge: protocol and client.
//!
//! The device side (`fdbd`) and the host side (`fdb`) exchange frames over
//! a byte stream (a virtio-console port under QEMU, a USB function on a
//! phone):
//!
//! ```text
//! 0xFD  kind:u8  len:u32le  payload[len]
//! ```
//!
//! The magic byte lets a reader resynchronise after a peer went away in
//! the middle of a frame. Requests and replies:
//!
//! | host → device | payload | device → host |
//! | --- | --- | --- |
//! | `H` hello | — | `H` device description |
//! | `X` exec | `sh -c` command line | `O` stdout, `E` stderr chunks, then `Z` exit status (i32) |
//! | `I` stdin | bytes for the running command | — |
//! | `C` close stdin | — | — |
//! | `W` write file | mode:u32, path len:u16, path, data | `K` or `F` message |
//! | `R` read file | path | `D` data or `F` message |
//! | `M` mkdir -p | mode:u32, path | `K` or `F` |
//! | `S` screenshot | — | `D` width:u32 height:u32 stride:u32 XRGB pixels, or `F` |

use std::io::{self, Read, Write};

pub const MAGIC: u8 = 0xFD;
/// Largest payload either side accepts.
pub const MAX_FRAME: usize = 256 << 20;

pub mod kind {
    pub const HELLO: u8 = b'H';
    pub const EXEC: u8 = b'X';
    pub const STDIN: u8 = b'I';
    pub const CLOSE_STDIN: u8 = b'C';
    pub const WRITE: u8 = b'W';
    pub const READ: u8 = b'R';
    pub const MKDIR: u8 = b'M';
    pub const SCREENSHOT: u8 = b'S';
    pub const STDOUT: u8 = b'O';
    pub const STDERR: u8 = b'E';
    pub const EXIT: u8 = b'Z';
    pub const OK: u8 = b'K';
    pub const FAIL: u8 = b'F';
    pub const DATA: u8 = b'D';
}

pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let mut head = [0u8; 6];
    head[0] = MAGIC;
    head[1] = kind;
    head[2..6].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    let mut frame = Vec::with_capacity(6 + payload.len());
    frame.extend_from_slice(&head);
    frame.extend_from_slice(payload);
    w.write_all(&frame)?;
    w.flush()
}

/// Read one frame, skipping bytes until the magic byte.
pub fn read_frame(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut b = [0u8; 1];
    loop {
        r.read_exact(&mut b)?;
        if b[0] == MAGIC {
            break;
        }
    }
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_le_bytes(head[1..5].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "fdb: frame too large"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

pub fn write_payload(mode: u32, path: &str, data: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + path.len() + data.len());
    p.extend_from_slice(&mode.to_le_bytes());
    p.extend_from_slice(&(path.len() as u16).to_le_bytes());
    p.extend_from_slice(path.as_bytes());
    p.extend_from_slice(data);
    p
}

/// Split a `W` payload into (mode, path, data).
pub fn parse_write(p: &[u8]) -> Option<(u32, String, &[u8])> {
    let mode = u32::from_le_bytes(p.get(..4)?.try_into().ok()?);
    let n = u16::from_le_bytes(p.get(4..6)?.try_into().ok()?) as usize;
    let path = String::from_utf8(p.get(6..6 + n)?.to_vec()).ok()?;
    Some((mode, path, &p[6 + n..]))
}

/// The host end of a connection.
pub struct Client<S: Read + Write> {
    stream: S,
}

/// What a command did.
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: i32,
}

fn fail(msg: Vec<u8>) -> io::Error {
    io::Error::other(String::from_utf8_lossy(&msg).into_owned())
}

impl<S: Read + Write> Client<S> {
    pub fn new(stream: S) -> Client<S> {
        Client { stream }
    }

    /// Exchange hellos; returns the device's description. Stale frames from
    /// an earlier session are skipped.
    pub fn hello(&mut self) -> io::Result<String> {
        write_frame(&mut self.stream, kind::HELLO, &[])?;
        loop {
            let (k, p) = read_frame(&mut self.stream)?;
            if k == kind::HELLO {
                return Ok(String::from_utf8_lossy(&p).into_owned());
            }
        }
    }

    /// Run `sh -c cmd`; `out` gets stdout/stderr chunks as they arrive
    /// (true = stderr). Returns the exit status.
    pub fn exec_stream(&mut self, cmd: &str, mut out: impl FnMut(bool, &[u8])) -> io::Result<i32> {
        write_frame(&mut self.stream, kind::EXEC, cmd.as_bytes())?;
        loop {
            let (k, p) = read_frame(&mut self.stream)?;
            match k {
                kind::STDOUT => out(false, &p),
                kind::STDERR => out(true, &p),
                kind::EXIT => return Ok(i32::from_le_bytes(p.get(..4).unwrap_or(&[0; 4]).try_into().unwrap())),
                kind::FAIL => return Err(fail(p)),
                _ => {}
            }
        }
    }

    pub fn exec(&mut self, cmd: &str) -> io::Result<Output> {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status =
            self.exec_stream(
                cmd,
                |err, b| if err { stderr.extend_from_slice(b) } else { stdout.extend_from_slice(b) },
            )?;
        Ok(Output { stdout, stderr, status })
    }

    pub fn stream(&mut self) -> &mut S {
        &mut self.stream
    }

    fn expect_ok(&mut self) -> io::Result<()> {
        loop {
            let (k, p) = read_frame(&mut self.stream)?;
            match k {
                kind::OK => return Ok(()),
                kind::FAIL => return Err(fail(p)),
                _ => {}
            }
        }
    }

    fn expect_data(&mut self) -> io::Result<Vec<u8>> {
        loop {
            let (k, p) = read_frame(&mut self.stream)?;
            match k {
                kind::DATA => return Ok(p),
                kind::FAIL => return Err(fail(p)),
                _ => {}
            }
        }
    }

    pub fn write_file(&mut self, path: &str, mode: u32, data: &[u8]) -> io::Result<()> {
        write_frame(&mut self.stream, kind::WRITE, &write_payload(mode, path, data))?;
        self.expect_ok()
    }

    pub fn read_file(&mut self, path: &str) -> io::Result<Vec<u8>> {
        write_frame(&mut self.stream, kind::READ, path.as_bytes())?;
        self.expect_data()
    }

    pub fn mkdir(&mut self, path: &str, mode: u32) -> io::Result<()> {
        let mut p = mode.to_le_bytes().to_vec();
        p.extend_from_slice(path.as_bytes());
        write_frame(&mut self.stream, kind::MKDIR, &p)?;
        self.expect_ok()
    }

    /// The display: (width, height, RGB pixels).
    pub fn screenshot(&mut self) -> io::Result<(u32, u32, Vec<u8>)> {
        write_frame(&mut self.stream, kind::SCREENSHOT, &[])?;
        let d = self.expect_data()?;
        if d.len() < 12 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short screenshot"));
        }
        let rd = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let (w, h, stride) = (rd(0), rd(4), rd(8) as usize);
        let px = &d[12..];
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let o = y * stride + x * 4;
                let p = px.get(o..o + 4).unwrap_or(&[0; 4]);
                rgb.extend_from_slice(&[p[2], p[1], p[0]]);
            }
        }
        Ok((w, h, rgb))
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Encode 8-bit RGB pixels as a PNG.
pub fn png(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for row in rgb.chunks(w as usize * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_resync() {
        let mut buf = vec![1, 2, 3]; // garbage before the first frame
        write_frame(&mut buf, kind::EXEC, b"echo hi").unwrap();
        write_frame(&mut buf, kind::EXIT, &7i32.to_le_bytes()).unwrap();
        let mut r = &buf[..];
        assert_eq!(read_frame(&mut r).unwrap(), (kind::EXEC, b"echo hi".to_vec()));
        assert_eq!(read_frame(&mut r).unwrap(), (kind::EXIT, 7i32.to_le_bytes().to_vec()));
    }

    #[test]
    fn write_payloads_parse_back() {
        let p = write_payload(0o755, "/tmp/x", b"data");
        let (mode, path, data) = parse_write(&p).unwrap();
        assert_eq!((mode, path.as_str(), data), (0o755, "/tmp/x", &b"data"[..]));
    }

    #[test]
    fn png_has_a_valid_crc() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        let p = png(1, 1, &[255, 0, 0]);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
    }
}
