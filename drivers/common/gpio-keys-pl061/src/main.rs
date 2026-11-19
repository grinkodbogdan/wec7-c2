//! ARM PL061 GPIO driver component for hardware keys.
//!
//! On the QEMU virt board the power key is wired to GPIO line 3 (QEMU
//! pulses it on `system_powerdown`). The driver holds the PL061 registers
//! and its interrupt, and reports key presses.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use flare_rt::abi::{BootInfo, CapKind, LABEL_NOTIFY};
use flare_rt::{println, sys};

flare_rt::entry!(main);

const GPIOIS: usize = 0x404; // interrupt sense: 0 = edge
const GPIOIBE: usize = 0x408; // both edges
const GPIOIEV: usize = 0x40c; // event: 1 = rising
const GPIOIE: usize = 0x410; // interrupt mask (1 = enabled)
const GPIOMIS: usize = 0x418; // masked status
const GPIOIC: usize = 0x41c; // clear

const POWER_LINE: u32 = 3;

struct Pl061(usize);

impl Pl061 {
    fn read(&self, r: usize) -> u32 {
        // SAFETY: the PL061 window the kernel mapped for us.
        unsafe { read_volatile((self.0 + r) as *const u32) }
    }
    fn write(&self, r: usize, v: u32) {
        // SAFETY: as above.
        unsafe { write_volatile((self.0 + r) as *mut u32, v) }
    }
}

fn main(bi: &'static BootInfo) -> i32 {
    let (Some(regs), Some(irq), Some(ep)) = (bi.find("regs"), bi.find("irq"), bi.find("events")) else {
        return 2;
    };
    if regs.kind() != CapKind::Mmio || irq.kind() != CapKind::Irq {
        return 2;
    }
    let gpio = Pl061(regs.vaddr as usize);
    if sys::irq_bind(irq.slot(), ep.slot(), 0).is_err() {
        return 3;
    }
    let bit = 1 << POWER_LINE;
    gpio.write(GPIOIS, gpio.read(GPIOIS) & !bit);
    gpio.write(GPIOIBE, gpio.read(GPIOIBE) & !bit);
    gpio.write(GPIOIEV, gpio.read(GPIOIEV) | bit);
    gpio.write(GPIOIC, 0xff);
    gpio.write(GPIOIE, gpio.read(GPIOIE) | bit);
    println!("gpio-keys: watching the power key (incarnation {})", bi.incarnation);
    loop {
        let Ok(msg) = sys::recv(ep.slot()) else { return 4 };
        if msg.label != LABEL_NOTIFY {
            let _ = sys::reply(u64::MAX, [0; 6]);
            continue;
        }
        let pending = gpio.read(GPIOMIS);
        gpio.write(GPIOIC, pending);
        if pending & bit != 0 {
            println!("gpio-keys: power key pressed");
        }
        let _ = sys::irq_ack(irq.slot());
    }
}
