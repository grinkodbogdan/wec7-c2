//! Burns CPU forever without yielding. The rest of the system must keep
//! running anyway: the framekernel preempts it on every timer tick.

#![no_std]
#![no_main]

use flare_rt::abi::BootInfo;
use flare_rt::println;

flare_rt::entry!(main);

fn main(_bi: &'static BootInfo) -> i32 {
    println!("spinner: starting a CPU-bound loop that never yields");
    let mut x: u64 = 0;
    loop {
        x = core::hint::black_box(x.wrapping_add(1));
    }
}
