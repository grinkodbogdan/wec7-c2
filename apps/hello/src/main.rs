//! A native Flare app: talks to the console service, demonstrates capability
//! derivation and revocation, and reads a line of input.

#![no_std]
#![no_main]

use flare_rt::abi::{BootInfo, Error, Rights};
use flare_rt::{console, println, sys};

flare_rt::entry!(main);

fn main(bi: &'static BootInfo) -> i32 {
    println!("hello: Hello from a native Flare component '{}' (incarnation {})", bi.name(), bi.incarnation);
    for c in bi.caps() {
        println!("hello:   holds slot {} = {} '{}'", c.slot, c.kind().name(), c.name());
    }

    // Delegate: derive a child of our console capability, use it, then
    // revoke it through the parent.
    let Some(parent) = console::slot() else { return 1 };
    let child = match sys::cap_derive(parent, Rights::SEND, 0) {
        Ok(s) => s,
        Err(e) => {
            println!("hello: UNEXPECTED derive failure {}", e);
            return 1;
        }
    };
    let _ = console::write_to(child, b"hello: this line travelled through a derived capability\n");
    if let Err(e) = sys::cap_revoke(parent) {
        println!("hello: UNEXPECTED revoke failure {}", e);
        return 1;
    }
    match console::write_to(child, b"hello: UNEXPECTED write through a revoked capability\n") {
        Err(Error::NoCap) => println!("hello: derived capability revoked -> {} as expected", Error::NoCap),
        other => println!("hello: UNEXPECTED result after revoke: {:?}", other),
    }

    println!("hello: type a line and press enter:");
    let mut line = [0u8; 64];
    let mut len = 0;
    let mut chunk = [0u8; 48];
    'outer: loop {
        let n = match console::poll_input(&mut chunk) {
            Ok(n) => n,
            Err(_) => return 1,
        };
        if n == 0 {
            sys::yield_now();
            continue;
        }
        for &b in &chunk[..n] {
            if b == b'\n' || b == b'\r' {
                break 'outer;
            }
            if len < line.len() {
                line[len] = b;
                len += 1;
            }
        }
    }
    let text = core::str::from_utf8(&line[..len]).unwrap_or("<invalid utf-8>");
    println!("hello: received input: {}", text);
    println!("hello: done, exiting");
    0
}
