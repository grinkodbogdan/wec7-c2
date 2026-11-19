//! Runtime for native Flare components.
//!
//! A component is a `#![no_std] #![no_main]` binary that declares its entry
//! point with [`entry!`]. It starts with nothing but the capabilities listed
//! in its [`BootInfo`] page; everything it does goes through [`sys`].

#![no_std]

pub use flare_abi as abi;
use flare_abi::BootInfo;

pub mod console;
pub mod sys;

/// Declare the component's main function: `fn(&'static BootInfo) -> i32`.
/// The return value becomes the exit code (non-zero counts as a failure).
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[no_mangle]
        #[link_section = ".text.start"]
        pub unsafe extern "C" fn _start(bootinfo: *const $crate::abi::BootInfo) -> ! {
            $crate::start(&*bootinfo, $main)
        }
    };
}

#[doc(hidden)]
pub fn start(bootinfo: &'static BootInfo, main: fn(&'static BootInfo) -> i32) -> ! {
    if bootinfo.magic != flare_abi::BOOTINFO_MAGIC {
        sys::exit(-1);
    }
    console::init(bootinfo);
    let code = main(bootinfo);
    sys::exit(code)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    sys::exit(101)
}
