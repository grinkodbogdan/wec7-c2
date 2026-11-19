//! ARM PL031 real-time clock driver component: serves `flare.rtc@1.0`.
//!
//! Holds only the PL031 register window and the receive side of the `rtc`
//! endpoint (plus a console client capability).

#![no_std]
#![no_main]

use core::ptr::read_volatile;
use flare_rt::abi::proto::rtc;
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY, MSG_WORDS};
use flare_rt::{println, sys};

flare_rt::entry!(main);

const DR: usize = 0x000;

/// (year, month, day, hour, minute, second) from seconds since 1970.
fn civil(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86400;
    let rem = secs % 86400;
    // Howard Hinnant's days-to-civil algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y as u64, m as u64, d as u64, rem / 3600, (rem / 60) % 60, rem % 60)
}

fn main(bi: &'static BootInfo) -> i32 {
    let (Some(regs), Some(ep)) = (bi.find("regs"), bi.find("rtc")) else { return 2 };
    if regs.kind() != CapKind::Mmio || ep.kind() != CapKind::Endpoint {
        return 2;
    }
    let base = regs.vaddr as usize;
    // SAFETY: the PL031 window the kernel mapped for us.
    let now = || unsafe { read_volatile((base + DR) as *const u32) } as u64;
    let (y, mo, d, h, mi, s) = civil(now());
    println!(
        "rtc-pl031: wall clock {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC (incarnation {})",
        y, mo, d, h, mi, s, bi.incarnation
    );
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 3 };
        if msg.label == LABEL_NOTIFY {
            continue;
        }
        let mut w = [0u64; MSG_WORDS];
        let label = match msg.label {
            rtc::GET_TIME => {
                w[0] = now();
                0
            }
            _ => u64::MAX,
        };
        let _ = sys::reply(label, w);
    }
}
