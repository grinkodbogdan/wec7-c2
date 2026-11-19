//! Reads the userdata disk through the storage driver and keeps reading
//! while the driver is restarted underneath it: each sector must still
//! come back right, from whichever incarnation answers.

#![no_std]
#![no_main]

use flare_rt::abi::proto::{block, console as bytes, rtc};
use flare_rt::abi::{BootInfo, Error, MSG_WORDS};
use flare_rt::{println, sys};

flare_rt::entry!(main);

fn now(rtc: u64) -> u64 {
    sys::call_retry(rtc, rtc::GET_TIME, [0; MSG_WORDS]).map(|m| m.words[0]).unwrap_or(0)
}

fn main(bi: &'static BootInfo) -> i32 {
    let (Some(st), Some(clock)) = (bi.find("storage"), bi.find("rtc")) else { return 2 };
    let (st, clock) = (st.slot(), clock.slot());
    let Ok(info) = sys::call_retry(st, block::INFO, [0; MSG_WORDS]) else { return 3 };
    let capacity = info.words[0];
    let first = sys::call_retry(st, block::READ, [0, 0, 0, 0, 0, 0]).map(|m| bytes::unpack(&m.words));
    let text = first.as_ref().map(|b| {
        let end = b.iter().position(|&c| c == b'\n' || c == 0).unwrap_or(b.len());
        core::str::from_utf8(&b[..end]).unwrap_or("?")
    });
    println!("storage-test: disk of {} sectors, sector 0 says: {}", capacity, text.unwrap_or("<read failed>"));

    // Read sectors 1..63 over and over until the driver has been restarted
    // under us and answered 200 more reads (or 20 s pass).
    let start_inc = info.words[1];
    let deadline = now(clock) + 20;
    let (mut verified, mut interrupted, mut after, mut wrong) = (0u64, 0u64, 0u64, 0u64);
    let mut restarted = false;
    let mut i = 0u64;
    while !(restarted && after >= 200) {
        if i.is_multiple_of(256) {
            if now(clock) > deadline {
                break;
            }
            if let Ok(m) = sys::call_retry(st, block::INFO, [0; MSG_WORDS]) {
                restarted |= m.words[1] != start_inc;
            }
        }
        let sector = 1 + i % 63;
        match sys::call(st, block::READ, [sector, 0, 0, 0, 0, 0]) {
            Err(Error::PeerDied | Error::NoReply) => {
                interrupted += 1;
                sys::yield_now();
                continue;
            }
            Err(_) => return 4,
            Ok(m) => {
                let b = bytes::unpack(&m.words);
                let mut want = *b"FLARE-SECTOR-0000";
                let mut n = sector;
                for k in (13..17).rev() {
                    want[k] = b'0' + (n % 10) as u8;
                    n /= 10;
                }
                if m.label as usize >= want.len() && b[..want.len()] == want {
                    verified += 1;
                    if restarted {
                        after += 1;
                    }
                } else {
                    wrong += 1;
                }
            }
        }
        i += 1;
    }
    println!(
        "storage-test: {} reads verified ({} after the driver restart), {} wrong, {} calls retried",
        verified, after, wrong, interrupted
    );
    if wrong == 0 && restarted && after >= 200 {
        println!("storage-test: all reads correct across a driver restart");
        0
    } else {
        println!("storage-test: FAILED");
        1
    }
}
