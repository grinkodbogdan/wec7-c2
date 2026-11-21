//! `/dev/dri/card0`: a DRM/KMS device over a `flare.display@1.0`
//! framebuffer in shared memory.
//!
//! The display driver (a user-space component) owns the panel and
//! publishes its mode in the shared header; this device gives Linux
//! programs the standard KMS interface over it: one CRTC, one encoder and
//! one connector, dumb buffers (shared memory objects) with PRIME
//! export, legacy modesetting and page flips with completion events.
//! A flip copies the buffer into the scanout memory. Rendering is done by
//! the clients (Mesa's software rasterizer through GBM); the connector
//! follows the driver: offline while it restarts, hot-plugged back after.

use super::abi::*;
use super::fs::{FileKind, EPOLLIN, EPOLLOUT};
use super::mm::{MemObj, MemRef};
use super::Kernel;
use crate::arch::mmu::phys_to_virt;
use crate::arch::timer;
use crate::mm::{page_ceil, PAGE_SIZE};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use flare_abi::proto::display as dp;

pub const CRTC_ID: u32 = 31;
pub const ENCODER_ID: u32 = 32;
pub const CONNECTOR_ID: u32 = 33;
/// mmap offsets of dumb buffers: handle << MAP_SHIFT.
const MAP_SHIFT: u32 = 28;
const FRAME_NS: u64 = 16_666_667;

/// A dumb buffer; its size is kept for imports and debugging.
#[allow(dead_code)]
struct Dumb {
    obj: MemRef,
    width: u32,
    height: u32,
    pitch: u32,
}

#[derive(Clone)]
struct Fb {
    obj: MemRef,
    width: u32,
    height: u32,
    pitch: u32,
    offset: u32,
}

/// One open of the device.
#[derive(Default)]
pub struct DrmFile {
    handles: BTreeMap<u32, Dumb>,
    next_handle: u32,
    /// Pending events: (due time, encoded drm_event).
    events: VecDeque<(u64, [u8; 32])>,
}

/// The device: framebuffers and the CRTC state are shared by all opens.
#[derive(Default)]
pub struct Drm {
    fbs: BTreeMap<u32, Fb>,
    next_fb: u32,
    scanout: Option<u32>,
    mode_set: bool,
    sequence: u32,
    last_vblank: u64,
    flips: u64,
    /// Presents copied to the panel, and the time spent copying (ns), for
    /// the profiler's report.
    pub presents: u64,
    pub present_ns: u64,
}

impl Drm {
    /// The framebuffer on the panel (the last one presented).
    pub fn scanout(&self) -> Option<u32> {
        self.scanout
    }
}

pub type DrmFileRef = Rc<RefCell<DrmFile>>;

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn wr32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn wr64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// The panel as the driver describes it, or None while it is offline.
#[derive(Clone, Copy)]
struct Panel {
    /// The mode clients render at: the panel's size divided by `scale`.
    width: u32,
    height: u32,
    /// Pixels drawn per client pixel, each way: 2 on large panels (a phone's
    /// 1080x1920), so software rendering fills a quarter of the pixels.
    scale: u32,
    stride: u32,
    fb_offset: u32,
    width_mm: u32,
    height_mm: u32,
    refresh: u32,
}

/// `struct drm_mode_modeinfo` for the panel (68 bytes).
fn modeinfo(p: &Panel) -> [u8; 68] {
    let mut m = [0u8; 68];
    let (w, h) = (p.width as u16, p.height as u16);
    let (htotal, vtotal) = (w + 48, h + 9);
    let refresh = p.refresh.max(1);
    wr32(&mut m, 0, htotal as u32 * vtotal as u32 * refresh / 1000);
    for (i, v) in [w, w + 16, w + 32, htotal, 0, h, h + 3, h + 6, vtotal, 0].iter().enumerate() {
        m[4 + i * 2..6 + i * 2].copy_from_slice(&v.to_le_bytes());
    }
    wr32(&mut m, 24, refresh);
    wr32(&mut m, 28, 0); // flags
    wr32(&mut m, 32, (1 << 3) | (1 << 6)); // PREFERRED | DRIVER
    let name = alloc::format!("{}x{}", p.width, p.height);
    m[36..36 + name.len().min(31)].copy_from_slice(&name.as_bytes()[..name.len().min(31)]);
    m
}

impl Kernel {
    fn panel(&self) -> Option<Panel> {
        let (pa, len) = *self.shm.get(crate::boot::LINUX_DISPLAY?)?;
        // SAFETY: the shared region is RAM reserved for the display, read
        // through the direct map; the driver owns the header.
        let h = unsafe { core::ptr::read_volatile(phys_to_virt(pa) as *const dp::Header) };
        let fb_end = h.fb_offset as u64 + h.stride as u64 * h.height as u64;
        let ok = h.magic == dp::MAGIC
            && h.state == dp::STATE_ONLINE
            && h.format == dp::XRGB8888
            && h.width > 0
            && h.height > 0
            && h.stride >= h.width * 4
            && fb_end <= len;
        let scale = if h.width >= 1000 && h.height >= 1000 { 2 } else { 1 };
        ok.then_some(Panel {
            width: h.width / scale,
            height: h.height / scale,
            scale,
            stride: h.stride,
            fb_offset: h.fb_offset,
            width_mm: h.width_mm,
            height_mm: h.height_mm,
            refresh: if h.refresh_hz == 0 { 60 } else { h.refresh_hz },
        })
    }

    fn drm(&mut self) -> &mut Drm {
        &mut self.lx().drm
    }

    /// Copy framebuffer `fb_id` to the panel.
    pub(crate) fn drm_present(&mut self, fb_id: u32) {
        // (the console has the screen: shown again on `console hide`)
        if crate::console::shown() {
            return;
        }
        let Some(p) = self.panel() else { return };
        let Some(fb) = self.drm().fbs.get(&fb_id).cloned() else { return };
        let t0 = timer::now_ns();
        let (pa, _) = self.shm[crate::boot::LINUX_DISPLAY.unwrap()];
        let dst = phys_to_virt(pa) + p.fb_offset as u64;
        let (w, h) = (fb.width.min(p.width) as u64 * 4, fb.height.min(p.height));
        let mut obj = fb.obj.borrow_mut();
        let mut rows = 0u32;
        // a scaled panel: each client row goes through here, then doubled
        let mut row: Vec<u32> = if p.scale > 1 { alloc::vec![0; (w / 4) as usize] } else { Vec::new() };
        for y in 0..h as u64 {
            let src_off = fb.offset as u64 + y * fb.pitch as u64;
            let mut done = 0u64;
            while done < w {
                let pos = src_off + done;
                let n = (PAGE_SIZE - pos % PAGE_SIZE).min(w - done);
                let d = if p.scale > 1 { row.as_mut_ptr() as u64 + done } else { dst + y * p.stride as u64 + done };
                match obj.page(pos / PAGE_SIZE) {
                    // SAFETY: both ranges are RAM inside their objects (or the row buffer).
                    Some(spa) => unsafe {
                        core::ptr::copy_nonoverlapping(
                            (phys_to_virt(spa) + pos % PAGE_SIZE) as *const u8,
                            d as *mut u8,
                            n as usize,
                        )
                    },
                    None => break,
                }
                done += n;
            }
            if done >= w {
                rows += 1;
            }
            if p.scale == 2 {
                // each pixel twice, the row twice
                let out = dst + 2 * y * p.stride as u64;
                // SAFETY: two rows of the panel's memory, inside the framebuffer.
                unsafe {
                    let d0 = out as *mut u64;
                    for (i, px) in row.iter().enumerate() {
                        core::ptr::write(d0.add(i), (*px as u64) << 32 | *px as u64);
                    }
                    core::ptr::copy_nonoverlapping(out as *const u8, (out + p.stride as u64) as *mut u8, row.len() * 8);
                }
            }
        }
        drop(obj);
        if self.drm().flips == 0 {
            // Said before the console goes quiet: what the first frame was.
            kprintln!(
                "drm: first frame on the panel: framebuffer {} {}x{} (pitch {}) to {}x{} (stride {}), {} of {} rows copied",
                fb_id,
                fb.width,
                fb.height,
                fb.pitch,
                p.width,
                p.height,
                p.stride,
                rows,
                h
            );
        }
        // The panel's display engine reads memory, not the CPU's caches.
        crate::arch::mmu::clean_dcache(dst, p.stride as u64 * h as u64 * p.scale as u64);
        // A UI owns the screen now: the kernel console stops drawing on it.
        crate::console::set_quiet(true);
        let d = self.drm();
        d.scanout = Some(fb_id);
        d.flips += 1;
        d.presents += 1;
        d.present_ns += timer::now_ns() - t0;
    }

    fn vblank_now(&mut self) -> (u32, u64) {
        let now = timer::now_ns();
        let d = self.drm();
        if now >= d.last_vblank + FRAME_NS {
            let n = (now - d.last_vblank) / FRAME_NS;
            d.sequence = d.sequence.wrapping_add(n as u32);
            d.last_vblank += n * FRAME_NS;
        }
        (d.sequence, now)
    }

    /// Queue a `drm_event_vblank` for the next vblank.
    fn drm_queue_event(&mut self, f: &DrmFileRef, kind: u32, user_data: u64) {
        let (seq, now) = self.vblank_now();
        let due = self.drm().last_vblank + FRAME_NS;
        let mut e = [0u8; 32];
        wr32(&mut e, 0, kind);
        wr32(&mut e, 4, 32);
        wr64(&mut e, 8, user_data);
        let t = due.max(now) / 1000;
        wr32(&mut e, 16, (t / 1_000_000) as u32);
        wr32(&mut e, 20, (t % 1_000_000) as u32);
        wr32(&mut e, 24, seq.wrapping_add(1));
        wr32(&mut e, 28, CRTC_ID);
        f.borrow_mut().events.push_back((due, e));
    }

    pub(crate) fn drm_open(&mut self) -> FileKind {
        FileKind::Drm(Rc::new(RefCell::new(DrmFile { next_handle: 1, ..Default::default() })))
    }

    pub(crate) fn drm_poll(&self, f: &DrmFileRef) -> u32 {
        let now = timer::now_ns();
        let ready = f.borrow().events.front().is_some_and(|(due, _)| *due <= now);
        (if ready { EPOLLIN } else { 0 }) | EPOLLOUT
    }

    pub(crate) fn drm_read(&mut self, f: &DrmFileRef, max: usize) -> Option<Vec<u8>> {
        let now = timer::now_ns();
        let mut f = f.borrow_mut();
        let mut out = Vec::new();
        while out.len() + 32 <= max {
            match f.events.front() {
                Some((due, _)) if *due <= now => out.extend_from_slice(&f.events.pop_front().unwrap().1),
                _ => break,
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// Backing of an mmap of the device at `off`.
    pub(crate) fn drm_mmap(&mut self, f: &DrmFileRef, off: u64) -> Option<(MemRef, u64)> {
        let handle = (off >> MAP_SHIFT) as u32;
        let f = f.borrow();
        f.handles.get(&handle).map(|d| (d.obj.clone(), off - ((handle as u64) << MAP_SHIFT)))
    }

    /// The handle of a PRIME-imported object, creating one if needed.
    fn drm_import(&mut self, f: &DrmFileRef, obj: MemRef) -> u32 {
        let mut f = f.borrow_mut();
        if let Some((&h, _)) = f.handles.iter().find(|(_, d)| Rc::ptr_eq(&d.obj, &obj)) {
            return h;
        }
        let h = f.next_handle;
        f.next_handle += 1;
        f.handles.insert(h, Dumb { obj, width: 0, height: 0, pitch: 0 });
        h
    }

    fn write_u(&mut self, c: usize, arg: u64, b: &[u8]) -> Result<i64, i64> {
        self.uwrite(c, arg, b).map(|_| 0)
    }

    /// Fill a user array of u32 ids if it has room.
    fn put_ids(&mut self, c: usize, ptr: u64, room: u32, ids: &[u32]) -> Result<(), i64> {
        if ptr != 0 && room as usize >= ids.len() {
            let b: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.uwrite(c, ptr, &b)?;
        }
        Ok(())
    }

    pub(crate) fn drm_ioctl(&mut self, c: usize, f: &DrmFileRef, req: u64, arg: u64) -> Result<i64, i64> {
        let size = ((req >> 16) & 0x3fff) as usize;
        if (req >> 8) & 0xff != 0x64 {
            return Err(ENOTTY);
        }
        let nr = req & 0xff;
        let mut b = if size > 0 { self.uread(c, arg, size)? } else { Vec::new() };
        let panel = self.panel();
        match nr {
            // DRM_IOCTL_VERSION
            0x00 => {
                wr32(&mut b, 0, 1);
                wr32(&mut b, 4, 0);
                wr32(&mut b, 8, 0);
                for (i, s) in [&b"flare"[..], b"20260101", b"Flare display (shared-memory KMS)"].iter().enumerate() {
                    let (len_off, ptr_off) = (16 + i * 16, 24 + i * 16);
                    let (room, ptr) = (rd64(&b, len_off) as usize, rd64(&b, ptr_off));
                    if ptr != 0 && room > 0 {
                        self.uwrite(c, ptr, &s[..s.len().min(room)])?;
                    }
                    wr64(&mut b, len_off, s.len() as u64);
                }
                self.write_u(c, arg, &b)
            }
            // GET_UNIQUE: no bus id
            0x01 => {
                wr64(&mut b, 0, 0);
                self.write_u(c, arg, &b)
            }
            // GET_MAGIC
            0x02 => {
                wr32(&mut b, 0, 0x464c);
                self.write_u(c, arg, &b)
            }
            // GEM_CLOSE
            0x09 => {
                let h = rd32(&b, 0);
                f.borrow_mut().handles.remove(&h).map(|_| 0).ok_or(EINVAL)
            }
            // GET_CAP
            0x0c => {
                let v = match rd64(&b, 0) {
                    0x1 => 1,  // DUMB_BUFFER
                    0x2 => 1,  // VBLANK_HIGH_CRTC
                    0x3 => 24, // DUMB_PREFERRED_DEPTH
                    0x4 => 0,  // DUMB_PREFER_SHADOW
                    0x5 => 3,  // PRIME: import | export
                    0x6 => 1,  // TIMESTAMP_MONOTONIC
                    0x8 | 0x9 => 64,
                    0x12 => 1, // CRTC_IN_VBLANK_EVENT
                    _ => 0,
                };
                wr64(&mut b, 8, v);
                self.write_u(c, arg, &b)
            }
            // SET_CLIENT_CAP: stereo, universal planes, atomic, ... not offered
            0x0d => Err(EINVAL),
            // AUTH_MAGIC, SET_MASTER, DROP_MASTER
            0x11 | 0x1e | 0x1f => Ok(0),
            // PRIME_HANDLE_TO_FD
            0x2d => {
                let h = rd32(&b, 0);
                let obj = f.borrow().handles.get(&h).map(|d| d.obj.clone()).ok_or(ENOENT)?;
                let cloexec = rd32(&b, 4) & O_CLOEXEC != 0;
                let file = super::fs::open_file(FileKind::MemFd(obj), O_RDWR, "dmabuf");
                let fd = self.proc(c).install_fd(file, cloexec, 0)?;
                wr32(&mut b, 8, fd as u32);
                self.write_u(c, arg, &b)
            }
            // PRIME_FD_TO_HANDLE
            0x2e => {
                let fd = rd32(&b, 8) as u64;
                let file = self.proc(c).file(fd)?;
                let obj = match &file.borrow().kind {
                    FileKind::MemFd(o) => o.clone(),
                    _ => return Err(EINVAL),
                };
                let h = self.drm_import(f, obj);
                wr32(&mut b, 0, h);
                self.write_u(c, arg, &b)
            }
            // WAIT_VBLANK
            0x3a => {
                let kind = rd32(&b, 0);
                let (seq, now) = self.vblank_now();
                if kind & 0x0400_0000 != 0 {
                    self.drm_queue_event(f, 0x01, rd64(&b, 8));
                }
                wr32(&mut b, 0, kind & 0x1f);
                wr32(&mut b, 4, seq);
                wr64(&mut b, 8, now / 1_000_000_000);
                wr64(&mut b, 16, now / 1000 % 1_000_000);
                self.write_u(c, arg, &b)
            }
            // MODE_GETRESOURCES
            0xa0 => {
                let fbs: Vec<u32> = self.drm().fbs.keys().copied().collect();
                self.put_ids(c, rd64(&b, 0), rd32(&b, 32), &fbs)?;
                self.put_ids(c, rd64(&b, 8), rd32(&b, 36), &[CRTC_ID])?;
                self.put_ids(c, rd64(&b, 16), rd32(&b, 40), &[CONNECTOR_ID])?;
                self.put_ids(c, rd64(&b, 24), rd32(&b, 44), &[ENCODER_ID])?;
                for (o, v) in
                    [(32, fbs.len() as u32), (36, 1), (40, 1), (44, 1), (48, 1), (52, 8192), (56, 1), (60, 8192)]
                {
                    wr32(&mut b, o, v);
                }
                self.write_u(c, arg, &b)
            }
            // MODE_GETCRTC
            0xa1 => {
                if rd32(&b, 12) != CRTC_ID {
                    return Err(ENOENT);
                }
                let d = self.drm();
                let (fb, set) = (d.scanout.unwrap_or(0), d.mode_set);
                wr32(&mut b, 16, fb);
                wr32(&mut b, 20, 0);
                wr32(&mut b, 24, 0);
                wr32(&mut b, 28, 0); // gamma size
                match (panel, set) {
                    (Some(p), true) => {
                        wr32(&mut b, 32, 1);
                        b[36..104].copy_from_slice(&modeinfo(&p));
                    }
                    _ => wr32(&mut b, 32, 0),
                }
                self.write_u(c, arg, &b)
            }
            // MODE_SETCRTC
            0xa2 => {
                if rd32(&b, 12) != CRTC_ID {
                    return Err(ENOENT);
                }
                let (fb, valid) = (rd32(&b, 16), rd32(&b, 32));
                if valid == 0 || fb == 0 {
                    let d = self.drm();
                    d.mode_set = false;
                    d.scanout = None;
                    return Ok(0);
                }
                if panel.is_none() {
                    return Err(EINVAL);
                }
                if !self.drm().fbs.contains_key(&fb) {
                    return Err(ENOENT);
                }
                self.drm().mode_set = true;
                self.drm_present(fb);
                Ok(0)
            }
            // MODE_CURSOR, MODE_CURSOR2: no hardware cursor
            0xa3 | 0xbb => Err(ENXIO),
            // MODE_GETGAMMA, MODE_SETGAMMA
            0xa4 | 0xa5 => Ok(0),
            // MODE_GETENCODER
            0xa6 => {
                if rd32(&b, 0) != ENCODER_ID {
                    return Err(ENOENT);
                }
                wr32(&mut b, 4, 2); // DRM_MODE_ENCODER_LVDS
                wr32(&mut b, 8, CRTC_ID);
                wr32(&mut b, 12, 1);
                wr32(&mut b, 16, 0);
                self.write_u(c, arg, &b)
            }
            // MODE_GETCONNECTOR
            0xa7 => {
                if rd32(&b, 48) != CONNECTOR_ID {
                    return Err(ENOENT);
                }
                let (modes_ptr, room_modes) = (rd64(&b, 8), rd32(&b, 32));
                self.put_ids(c, rd64(&b, 0), rd32(&b, 40), &[ENCODER_ID])?;
                let n_modes = if let Some(p) = panel {
                    if modes_ptr != 0 && room_modes >= 1 {
                        self.uwrite(c, modes_ptr, &modeinfo(&p))?;
                    }
                    1
                } else {
                    0
                };
                wr32(&mut b, 32, n_modes);
                wr32(&mut b, 36, 0); // props
                wr32(&mut b, 40, 1); // encoders
                wr32(&mut b, 44, ENCODER_ID);
                wr32(&mut b, 52, 7); // DRM_MODE_CONNECTOR_LVDS (a phone panel)
                wr32(&mut b, 56, 1);
                wr32(&mut b, 60, if panel.is_some() { 1 } else { 2 });
                wr32(&mut b, 64, panel.map(|p| p.width_mm).unwrap_or(0));
                wr32(&mut b, 68, panel.map(|p| p.height_mm).unwrap_or(0));
                wr32(&mut b, 72, 1); // SubPixelUnknown
                self.write_u(c, arg, &b)
            }
            // MODE_GETPROPERTY, SETPROPERTY, GETPROPBLOB, OBJ_SETPROPERTY
            0xaa | 0xab | 0xac | 0xba => Err(EINVAL),
            // MODE_GETFB
            0xad => {
                let id = rd32(&b, 0);
                let fb = self.drm().fbs.get(&id).cloned().ok_or(ENOENT)?;
                wr32(&mut b, 4, fb.width);
                wr32(&mut b, 8, fb.height);
                wr32(&mut b, 12, fb.pitch);
                wr32(&mut b, 16, 32);
                wr32(&mut b, 20, 24);
                wr32(&mut b, 24, 0);
                self.write_u(c, arg, &b)
            }
            // MODE_ADDFB (legacy: bpp/depth) and MODE_ADDFB2 (fourcc)
            0xae | 0xb8 => {
                let (w, h) = (rd32(&b, 4), rd32(&b, 8));
                let (handle, pitch, offset) = if nr == 0xae {
                    if rd32(&b, 16) != 32 {
                        return Err(EINVAL);
                    }
                    (rd32(&b, 24), rd32(&b, 12), 0)
                } else {
                    // XRGB8888 / ARGB8888 only
                    if ![dp::XRGB8888, 0x3432_5241].contains(&rd32(&b, 12)) {
                        return Err(EINVAL);
                    }
                    (rd32(&b, 20), rd32(&b, 36), rd32(&b, 52))
                };
                let obj = f.borrow().handles.get(&handle).map(|d| d.obj.clone()).ok_or(ENOENT)?;
                if w == 0 || h == 0 || pitch < w * 4 || offset as u64 + pitch as u64 * h as u64 > obj.borrow().size {
                    return Err(EINVAL);
                }
                let d = self.drm();
                d.next_fb = d.next_fb.max(100) + 1;
                let id = d.next_fb;
                d.fbs.insert(id, Fb { obj, width: w, height: h, pitch, offset });
                wr32(&mut b, 0, id);
                self.write_u(c, arg, &b)
            }
            // MODE_RMFB
            0xaf => {
                let id = rd32(&b, 0);
                let d = self.drm();
                if d.scanout == Some(id) {
                    d.scanout = None;
                }
                d.fbs.remove(&id).map(|_| 0).ok_or(ENOENT)
            }
            // MODE_PAGE_FLIP
            0xb0 => {
                let (crtc, fb, flags, user) = (rd32(&b, 0), rd32(&b, 4), rd32(&b, 8), rd64(&b, 16));
                if crtc != CRTC_ID {
                    return Err(ENOENT);
                }
                if !self.drm().mode_set || panel.is_none() {
                    return Err(EINVAL);
                }
                if !self.drm().fbs.contains_key(&fb) {
                    return Err(ENOENT);
                }
                if f.borrow().events.iter().any(|(_, e)| rd32(e, 0) == 0x02) {
                    return Err(EBUSY);
                }
                self.drm_present(fb);
                if flags & 1 != 0 {
                    self.drm_queue_event(f, 0x02, user);
                }
                Ok(0)
            }
            // MODE_DIRTYFB: front-buffer rendering
            0xb1 => {
                let id = rd32(&b, 0);
                if self.drm().scanout == Some(id) {
                    self.drm_present(id);
                }
                Ok(0)
            }
            // MODE_CREATE_DUMB
            0xb2 => {
                let (h, w, bpp) = (rd32(&b, 0), rd32(&b, 4), rd32(&b, 8));
                if w == 0 || h == 0 || w > 8192 || h > 8192 || !(8..=32).contains(&bpp) {
                    return Err(EINVAL);
                }
                let pitch = (w * bpp.div_ceil(8)).next_multiple_of(64);
                let size = page_ceil(pitch as u64 * h as u64);
                let obj = Rc::new(RefCell::new(MemObj::new(size)));
                let mut fm = f.borrow_mut();
                let handle = fm.next_handle;
                fm.next_handle += 1;
                fm.handles.insert(handle, Dumb { obj, width: w, height: h, pitch });
                wr32(&mut b, 16, handle);
                wr32(&mut b, 20, pitch);
                wr64(&mut b, 24, size);
                drop(fm);
                self.write_u(c, arg, &b)
            }
            // MODE_MAP_DUMB
            0xb3 => {
                let h = rd32(&b, 0);
                if !f.borrow().handles.contains_key(&h) {
                    return Err(ENOENT);
                }
                wr64(&mut b, 8, (h as u64) << MAP_SHIFT);
                self.write_u(c, arg, &b)
            }
            // MODE_DESTROY_DUMB
            0xb4 => {
                let h = rd32(&b, 0);
                f.borrow_mut().handles.remove(&h).map(|_| 0).ok_or(EINVAL)
            }
            // MODE_GETPLANERESOURCES: no planes beyond the primary
            0xb5 => {
                wr32(&mut b, 8, 0);
                self.write_u(c, arg, &b)
            }
            0xb6 => Err(ENOENT),
            // MODE_OBJ_GETPROPERTIES: none
            0xb9 => {
                wr32(&mut b, 16, 0);
                self.write_u(c, arg, &b)
            }
            _ => Err(EINVAL),
        }
    }

    /// /dev/fb0: the panel's pixels, read and written directly.
    fn fb_region(&self) -> Option<(u64, u64)> {
        let p = self.panel()?;
        let (pa, _) = *self.shm.get(crate::boot::LINUX_DISPLAY?)?;
        Some((phys_to_virt(pa) + p.fb_offset as u64, p.stride as u64 * (p.height * p.scale) as u64))
    }

    pub(crate) fn fb_read(&self, pos: u64, max: usize) -> Vec<u8> {
        let Some((va, len)) = self.fb_region() else { return Vec::new() };
        let n = (max as u64).min(len.saturating_sub(pos)) as usize;
        // SAFETY: inside the display memory, through the direct map.
        unsafe { core::slice::from_raw_parts((va + pos) as *const u8, n).to_vec() }
    }

    pub(crate) fn fb_write(&self, pos: u64, data: &[u8]) -> usize {
        let Some((va, len)) = self.fb_region() else { return 0 };
        let n = (data.len() as u64).min(len.saturating_sub(pos)) as usize;
        // SAFETY: as above.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), (va + pos) as *mut u8, n) };
        crate::arch::mmu::clean_dcache(va + pos, n as u64);
        n
    }

    /// FBIOGET_VSCREENINFO and FBIOGET_FSCREENINFO.
    pub(crate) fn fb_ioctl(&mut self, c: usize, req: u64, arg: u64) -> Result<i64, i64> {
        let p = self.panel().ok_or(ENODEV)?;
        match req & 0xffff {
            0x4600 => {
                let mut v = [0u8; 160];
                // /dev/fb0 is the panel's own memory: its real size
                let (pw, ph) = (p.width * p.scale, p.height * p.scale);
                for (i, x) in [pw, ph, pw, ph, 0, 0, 32, 0].iter().enumerate() {
                    wr32(&mut v, i * 4, *x);
                }
                // red, green, blue, transp bitfields: offset, length, msb_right
                for (i, (off, len)) in [(16, 8), (8, 8), (0, 8), (24, 0)].iter().enumerate() {
                    wr32(&mut v, 32 + i * 12, *off);
                    wr32(&mut v, 36 + i * 12, *len);
                }
                wr32(&mut v, 88, p.height_mm);
                wr32(&mut v, 92, p.width_mm);
                self.uwrite(c, arg, &v).map(|_| 0)
            }
            0x4602 => {
                let mut v = [0u8; 80];
                v[..9].copy_from_slice(b"flare-shm");
                wr32(&mut v, 24, p.stride * p.height * p.scale);
                wr32(&mut v, 36, 2); // FB_VISUAL_TRUECOLOR
                wr32(&mut v, 48, p.stride);
                self.uwrite(c, arg, &v).map(|_| 0)
            }
            0x4601 => Ok(0), // FBIOPUT_VSCREENINFO: the mode is fixed
            _ => Err(EINVAL),
        }
    }
}
