//! A plain Rust `std` program built for `aarch64-unknown-linux-musl`. It
//! runs unmodified on Flare's Linux personality and checks the parts of the
//! ABI that std relies on: args, heap, files, time, FP/SIMD, process spawn
//! with pipes.

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("std-test: args = {:?}", args);

    let squares: HashMap<u64, u64> = (0..1000).map(|i| (i, i * i)).collect();
    println!("std-test: HashMap with {} entries, 999^2 = {}", squares.len(), squares[&999]);

    let v: Vec<u64> = (1..=100_000).collect();
    println!("std-test: sum of 1..=100000 = {}", v.iter().sum::<u64>());

    std::fs::write("/tmp/std.txt", "rust std on flare").expect("write");
    println!("std-test: file round-trip: {}", std::fs::read_to_string("/tmp/std.txt").expect("read"));

    let x: f64 = std::hint::black_box(2.0);
    println!("std-test: sqrt(2) = {:.6}", x.sqrt());

    let t = Instant::now();
    std::thread::sleep(Duration::from_millis(50));
    println!("std-test: slept >= 50 ms: {}", t.elapsed() >= Duration::from_millis(50));

    match Command::new("/bin/echo").arg("spawned through fork/exec").output() {
        Ok(o) => println!(
            "std-test: child said '{}' (status {})",
            String::from_utf8_lossy(&o.stdout).trim(),
            o.status.code().unwrap_or(-1)
        ),
        Err(e) => println!("std-test: spawn failed: {}", e),
    }
    println!("std-test: ok");
}
