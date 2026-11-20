//! Keys and switches on Qualcomm TLMM GPIO pins (the Xiaomi Mi A1's volume-up
//! key and hall sensor), as one Linux evdev device.
//!
//! Each pin is granted as its own register window, named for what it reports:
//!
//! * `key-<code>=mmio:<pin>`: an `EV_KEY` with Linux key code `<code>`
//!   (115 = KEY_VOLUMEUP);
//! * `switch-<code>=mmio:<pin>`: an `EV_SW` (0 = SW_LID, a hall sensor).
//!
//! The pins are active low and polled every 20 ms: on MSM8953 their interrupts
//! go through the TLMM summary line, which the full pin controller (Linux's,
//! in the driver host) owns. The approach and register layout follow QRT's
//! src/arch/arm64/msm.c: one 4 KiB block per pin, CFG at +0, IN_OUT at +4
//! (bit 0 = level).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use flare_rt::abi::proto::input::{Abs, Describe, Event};
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY};
use flare_rt::{println, sys};

flare_rt::entry!(main);

const CFG: usize = 0x0;
const IN_OUT: usize = 0x4;
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_SW: u16 = 5;
const POLL_NS: u64 = 20_000_000;
const MAX_PINS: usize = 8;

#[derive(Clone, Copy)]
struct Pin {
    regs: usize,
    kind: u16,
    code: u16,
    down: bool,
}

fn parse_code(s: &str) -> Option<u16> {
    let mut v: u16 = 0;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((b - b'0') as u16)?;
    }
    Some(v)
}

fn level(regs: usize) -> bool {
    // SAFETY: the pin's TLMM block, mapped for us as device memory.
    unsafe { read_volatile((regs + IN_OUT) as *const u32) & 1 != 0 }
}

fn main(bi: &'static BootInfo) -> i32 {
    let (Some(input), Some(ep)) = (bi.find("input"), bi.find("events")) else { return 2 };
    if input.kind() != CapKind::Input || ep.kind() != CapKind::Endpoint {
        return 2;
    }
    let mut pins = [Pin { regs: 0, kind: 0, code: 0, down: false }; MAX_PINS];
    let mut n = 0;
    let mut d = Describe {
        name: [0; 64],
        bustype: 0x19, // BUS_HOST
        vendor: 0,
        product: 0,
        version: 1,
        ev_bits: 1 << EV_SYN,
        key_bits: [0; 96],
        rel_bits: [0; 2],
        abs_bits: [0; 8],
        prop_bits: [0; 4],
        abs: [Abs::default(); 64],
    };
    let name = b"gpio-keys";
    d.name[..name.len()].copy_from_slice(name);
    for c in bi.caps() {
        if c.kind() != CapKind::Mmio || n == MAX_PINS {
            continue;
        }
        let (kind, code) = if let Some(k) = c.name().strip_prefix("key-") {
            (EV_KEY, parse_code(k))
        } else if let Some(s) = c.name().strip_prefix("switch-") {
            (EV_SW, parse_code(s))
        } else {
            continue;
        };
        let Some(code) = code else { continue };
        let regs = c.vaddr as usize;
        // A GPIO input with the pull-up the key needs (as mainline's
        // gpio_key_default pin configuration): function 0, pull up, no output.
        // SAFETY: the pin's own configuration register.
        unsafe {
            let cfg = read_volatile((regs + CFG) as *const u32);
            write_volatile((regs + CFG) as *mut u32, (cfg & !(0x3 | (0xf << 2) | (1 << 9))) | 0x3);
        }
        d.ev_bits |= 1 << kind;
        if kind == EV_KEY && (code as usize) < 96 * 8 {
            d.key_bits[code as usize / 8] |= 1 << (code % 8);
        }
        pins[n] = Pin { regs, kind, code, down: !level(regs) };
        n += 1;
    }
    if n == 0 {
        return 3;
    }
    // SAFETY: Describe is plain old data.
    let bytes =
        unsafe { core::slice::from_raw_parts(&d as *const Describe as *const u8, core::mem::size_of::<Describe>()) };
    if sys::input_report(input.slot(), 0, bytes).is_err() {
        return 4;
    }
    // Switches report their state once at start.
    let mut evs = [Event::default(); 2 * MAX_PINS + 1];
    let mut m = 0;
    for p in &pins[..n] {
        if p.kind == EV_SW {
            evs[m] = Event { kind: EV_SW, code: p.code, value: p.down as i32 };
            m += 1;
        }
    }
    report(input.slot(), &mut evs, m);
    println!("gpio-keys-tlmm: {} pin(s) polled (incarnation {})", n, bi.incarnation);
    if sys::notify_after(ep.slot(), 0, POLL_NS).is_err() {
        return 5;
    }
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 6 };
        if msg.label != LABEL_NOTIFY {
            let _ = sys::reply(u64::MAX, [0; 6]);
            continue;
        }
        let mut m = 0;
        for p in pins[..n].iter_mut() {
            let down = !level(p.regs);
            if down != p.down {
                p.down = down;
                evs[m] = Event { kind: p.kind, code: p.code, value: down as i32 };
                m += 1;
                if p.kind == EV_KEY && down {
                    println!("gpio-keys-tlmm: key {} pressed", p.code);
                }
            }
        }
        report(input.slot(), &mut evs, m);
        let _ = sys::notify_after(ep.slot(), 0, POLL_NS);
    }
}

fn report(input: u64, evs: &mut [Event], m: usize) {
    if m == 0 {
        return;
    }
    evs[m] = Event { kind: EV_SYN, code: 0, value: 0 };
    // SAFETY: Event is plain old data.
    let bytes = unsafe { core::slice::from_raw_parts(evs.as_ptr() as *const u8, (m + 1) * 8) };
    let _ = sys::input_report(input, 1, bytes);
}
