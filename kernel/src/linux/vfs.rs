//! In-memory filesystem for the Linux personality.
//!
//! Populated at boot from the initramfs embedded in the kernel image
//! (format below), plus `/dev` nodes and a small `/proc`. Everything lives
//! in RAM; writes are allowed anywhere and lost at power-off.
//!
//! Initramfs format (little endian), repeated until the end:
//! `kind: u8 (1 file, 2 dir, 3 symlink)`, `mode: u32`, `path_len: u16`,
//! `data_len: u32`, `path`, `data` (file contents or symlink target).

use super::abi::*;
use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

pub type NodeRef = Rc<RefCell<Node>>;

/// Character devices (the `rdev` of `/dev` nodes).
pub const DEV_NULL: u64 = (1 << 8) | 3;
pub const DEV_ZERO: u64 = (1 << 8) | 5;
pub const DEV_URANDOM: u64 = (1 << 8) | 9;
pub const DEV_TTY: u64 = 5 << 8;
pub const DEV_CONSOLE: u64 = (5 << 8) | 1;
pub const DEV_KMSG: u64 = (1 << 8) | 11;
/// Binder devices (misc major 10).
pub const DEV_BINDER: u64 = (10 << 8) | 60;
pub const DEV_HWBINDER: u64 = (10 << 8) | 61;
pub const DEV_VNDBINDER: u64 = (10 << 8) | 62;
/// Flare's component supervisor (only with `[linux] control = true`).
pub const DEV_FLARECTL: u64 = (10 << 8) | 63;
/// DRM major 226, minor 0.
pub const DEV_DRI_CARD0: u64 = 226 << 8;
/// evdev: major 13, minors from 64.
pub const DEV_INPUT_EVENT0: u64 = (13 << 8) | 64;
/// Serial devices: the hvc major (229), one minor each.
pub const DEV_SERIAL0: u64 = 229 << 8;
/// fbdev view of the display: major 29, minor 0.
pub const DEV_FB0: u64 = 29 << 8;

pub struct Node {
    pub ino: u64,
    pub mode: u32,
    pub rdev: u64,
    /// File contents or symlink target. Initramfs files borrow the archive
    /// embedded in the kernel image and are copied on first write.
    pub data: Cow<'static, [u8]>,
    pub children: BTreeMap<String, NodeRef>,
    pub mtime: u64,
    /// Pages shared by MAP_SHARED mappings of this file.
    pub shm: Option<super::mm::MemRef>,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
    pub fn is_symlink(&self) -> bool {
        self.mode & S_IFMT == S_IFLNK
    }
    pub fn is_reg(&self) -> bool {
        self.mode & S_IFMT == S_IFREG
    }
}

pub struct Vfs {
    pub root: NodeRef,
    next_ino: u64,
}

pub struct Resolved {
    pub node: NodeRef,
    pub path: String,
}

/// Normalise `path` against `cwd` into an absolute path without `.`/`..`.
pub fn absolute(cwd: &str, path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let full: String = if path.starts_with('/') { path.to_string() } else { alloc::format!("{}/{}", cwd, path) };
    for p in full.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let mut out = String::from("/");
    out.push_str(&parts.join("/"));
    out
}

pub fn split_parent(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("/", path),
    }
}

impl Vfs {
    pub fn new() -> Vfs {
        let mut v = Vfs { root: Rc::new(RefCell::new(Node::dir(1, 0o755))), next_ino: 2 };
        for d in ["/dev", "/dev/shm", "/proc", "/tmp", "/root", "/etc", "/bin"] {
            let _ = v.mkdir_p(d, 0o755);
        }
        for (name, rdev, mode) in [
            ("null", DEV_NULL, 0o666),
            ("zero", DEV_ZERO, 0o666),
            ("urandom", DEV_URANDOM, 0o444),
            ("random", DEV_URANDOM, 0o444),
            ("tty", DEV_TTY, 0o666),
            ("console", DEV_CONSOLE, 0o600),
            ("kmsg", DEV_KMSG, 0o644),
            ("binder", DEV_BINDER, 0o666),
            ("hwbinder", DEV_HWBINDER, 0o666),
            ("vndbinder", DEV_VNDBINDER, 0o666),
        ] {
            let n = v.new_node(S_IFCHR | mode);
            n.borrow_mut().rdev = rdev;
            let dev = v.lookup("/", "/dev", true).unwrap().node;
            dev.borrow_mut().children.insert(name.to_string(), n);
        }
        for name in super::proc_files() {
            let n = v.new_node(S_IFREG | 0o444);
            let proc_dir = v.lookup("/", "/proc", true).unwrap().node;
            proc_dir.borrow_mut().children.insert(name.to_string(), n);
        }
        let n = v.new_node(S_IFLNK | 0o777);
        n.borrow_mut().data = Cow::Borrowed(b"/proc/self");
        let self_dir = v.new_node(S_IFDIR | 0o555);
        let exe = v.new_node(S_IFLNK | 0o777);
        exe.borrow_mut().data = Cow::Borrowed(b"/bin/busybox");
        self_dir.borrow_mut().children.insert("exe".to_string(), exe);
        let proc_dir = v.lookup("/", "/proc", true).unwrap().node;
        proc_dir.borrow_mut().children.insert("self".to_string(), self_dir);
        v
    }

    /// Add a character device node under /dev.
    pub fn add_device(&mut self, name: &str, rdev: u64, perm: u32) {
        let n = self.new_node(S_IFCHR | perm);
        n.borrow_mut().rdev = rdev;
        if let Ok(dev) = self.lookup("/", "/dev", true) {
            dev.node.borrow_mut().children.insert(name.to_string(), n);
        }
    }

    /// A detached directory of empty regular files (for generated /proc
    /// directories; their contents are produced when opened).
    pub fn synthetic_dir(names: &[&str]) -> NodeRef {
        let d = Rc::new(RefCell::new(Node::dir(0x4_0000_0000, 0o555)));
        for n in names {
            let mut f = Node::dir(0x4_0000_0001, 0o444);
            f.mode = S_IFREG | 0o444;
            d.borrow_mut().children.insert(n.to_string(), Rc::new(RefCell::new(f)));
        }
        d
    }

    /// Add a character device node `dir/name`, creating `dir`.
    pub fn add_device_at(&mut self, dir: &str, name: &str, rdev: u64, perm: u32) {
        let n = self.new_node(S_IFCHR | perm);
        n.borrow_mut().rdev = rdev;
        if let Ok(d) = self.mkdir_p(dir, 0o755) {
            d.borrow_mut().children.insert(name.to_string(), n);
        }
    }

    fn new_node(&mut self, mode: u32) -> NodeRef {
        let ino = self.next_ino;
        self.next_ino += 1;
        let mut n = Node::dir(ino, 0);
        n.mode = mode;
        Rc::new(RefCell::new(n))
    }

    /// Unpack an initramfs archive. Returns the number of entries.
    pub fn unpack(&mut self, mut a: &'static [u8]) -> Result<usize, &'static str> {
        let mut count = 0;
        while !a.is_empty() {
            if a.len() < 11 {
                return Err("truncated initramfs header");
            }
            let kind = a[0];
            let mode = u32::from_le_bytes(a[1..5].try_into().unwrap());
            let plen = u16::from_le_bytes(a[5..7].try_into().unwrap()) as usize;
            let dlen = u32::from_le_bytes(a[7..11].try_into().unwrap()) as usize;
            a = &a[11..];
            if a.len() < plen + dlen {
                return Err("truncated initramfs entry");
            }
            let path = core::str::from_utf8(&a[..plen]).map_err(|_| "bad path")?;
            let data = &a[plen..plen + dlen];
            a = &a[plen + dlen..];
            match kind {
                2 => {
                    self.mkdir_p(path, mode & 0o7777).map_err(|_| "cannot create directory")?;
                }
                1 | 3 => {
                    let (parent, name) = split_parent(path);
                    let dir = self.mkdir_p(parent, 0o755).map_err(|_| "cannot create directory")?;
                    let n = self.new_node(if kind == 1 { S_IFREG | (mode & 0o7777) } else { S_IFLNK | 0o777 });
                    n.borrow_mut().data = Cow::Borrowed(data);
                    dir.borrow_mut().children.insert(name.to_string(), n);
                }
                _ => return Err("unknown initramfs entry kind"),
            }
            count += 1;
        }
        Ok(count)
    }

    pub fn mkdir_p(&mut self, path: &str, mode: u32) -> Result<NodeRef, i64> {
        let mut cur = self.root.clone();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let next = cur.borrow().children.get(part).cloned();
            cur = match next {
                Some(n) if n.borrow().is_dir() => n,
                Some(_) => return Err(ENOTDIR),
                None => {
                    let n = self.new_node(S_IFDIR | mode);
                    cur.borrow_mut().children.insert(part.to_string(), n.clone());
                    n
                }
            };
        }
        Ok(cur)
    }

    /// Resolve a path. `follow` controls whether a final symlink is followed.
    pub fn lookup(&self, cwd: &str, path: &str, follow: bool) -> Result<Resolved, i64> {
        self.lookup_depth(cwd, path, follow, 0)
    }

    fn lookup_depth(&self, cwd: &str, path: &str, follow: bool, depth: u32) -> Result<Resolved, i64> {
        if depth > 8 {
            return Err(ELOOP);
        }
        if path.is_empty() {
            return Err(ENOENT);
        }
        let abs = absolute(cwd, path);
        let parts: Vec<&str> = abs.split('/').filter(|p| !p.is_empty()).collect();
        let mut cur = self.root.clone();
        let mut walked = String::new();
        for (i, part) in parts.iter().enumerate() {
            if !cur.borrow().is_dir() {
                return Err(ENOTDIR);
            }
            let next = cur.borrow().children.get(*part).cloned().ok_or(ENOENT)?;
            let last = i + 1 == parts.len();
            if next.borrow().is_symlink() && (!last || follow) {
                let target = String::from_utf8_lossy(&next.borrow().data).into_owned();
                let base = if walked.is_empty() { "/".to_string() } else { walked.clone() };
                let rest: Vec<&str> = parts[i + 1..].to_vec();
                let mut new_path = absolute(&base, &target);
                if !rest.is_empty() {
                    new_path.push('/');
                    new_path.push_str(&rest.join("/"));
                }
                return self.lookup_depth("/", &new_path, follow, depth + 1);
            }
            walked.push('/');
            walked.push_str(part);
            cur = next;
        }
        Ok(Resolved { node: cur, path: if walked.is_empty() { "/".to_string() } else { walked } })
    }

    /// Create a regular file (or return the existing one).
    pub fn create(&mut self, cwd: &str, path: &str, mode: u32, excl: bool) -> Result<Resolved, i64> {
        match self.lookup(cwd, path, true) {
            Ok(r) if excl => {
                drop(r);
                Err(EEXIST)
            }
            Ok(r) => Ok(r),
            Err(ENOENT) => {
                let abs = absolute(cwd, path);
                let (parent, name) = split_parent(&abs);
                let dir = self.lookup("/", parent, true)?;
                if !dir.node.borrow().is_dir() {
                    return Err(ENOTDIR);
                }
                if name.is_empty() {
                    return Err(EISDIR);
                }
                let n = self.new_node(S_IFREG | (mode & 0o7777));
                dir.node.borrow_mut().children.insert(name.to_string(), n.clone());
                Ok(Resolved { node: n, path: abs })
            }
            Err(e) => Err(e),
        }
    }

    pub fn mkdir(&mut self, cwd: &str, path: &str, mode: u32) -> Result<(), i64> {
        if self.lookup(cwd, path, false).is_ok() {
            return Err(EEXIST);
        }
        let abs = absolute(cwd, path);
        let (parent, name) = split_parent(&abs);
        let dir = self.lookup("/", parent, true)?;
        if !dir.node.borrow().is_dir() {
            return Err(ENOTDIR);
        }
        let n = self.new_node(S_IFDIR | (mode & 0o7777));
        dir.node.borrow_mut().children.insert(name.to_string(), n);
        Ok(())
    }

    pub fn symlink(&mut self, cwd: &str, target: &str, path: &str) -> Result<(), i64> {
        if self.lookup(cwd, path, false).is_ok() {
            return Err(EEXIST);
        }
        let abs = absolute(cwd, path);
        let (parent, name) = split_parent(&abs);
        let dir = self.lookup("/", parent, true)?;
        let n = self.new_node(S_IFLNK | 0o777);
        n.borrow_mut().data = Cow::Owned(target.as_bytes().to_vec());
        dir.node.borrow_mut().children.insert(name.to_string(), n);
        Ok(())
    }

    pub fn unlink(&mut self, cwd: &str, path: &str, dir_wanted: bool) -> Result<(), i64> {
        let abs = absolute(cwd, path);
        let (parent, name) = split_parent(&abs);
        let dir = self.lookup("/", parent, true)?;
        let node = dir.node.borrow().children.get(name).cloned().ok_or(ENOENT)?;
        let is_dir = node.borrow().is_dir();
        if dir_wanted && !is_dir {
            return Err(ENOTDIR);
        }
        if !dir_wanted && is_dir {
            return Err(EISDIR);
        }
        if is_dir && !node.borrow().children.is_empty() {
            return Err(ENOTEMPTY);
        }
        dir.node.borrow_mut().children.remove(name);
        Ok(())
    }

    pub fn rename(&mut self, cwd: &str, from: &str, to: &str) -> Result<(), i64> {
        let (fa, ta) = (absolute(cwd, from), absolute(cwd, to));
        let (fp, fname) = split_parent(&fa);
        let (tp, tname) = split_parent(&ta);
        let fdir = self.lookup("/", fp, true)?;
        let tdir = self.lookup("/", tp, true)?;
        let node = fdir.node.borrow_mut().children.remove(fname).ok_or(ENOENT)?;
        tdir.node.borrow_mut().children.insert(tname.to_string(), node);
        Ok(())
    }
}

impl Node {
    fn dir(ino: u64, perm: u32) -> Node {
        Node {
            ino,
            mode: S_IFDIR | perm,
            rdev: 0,
            data: Cow::Borrowed(&[]),
            children: BTreeMap::new(),
            mtime: 0,
            shm: None,
        }
    }
}
