#![doc = include_str!("../README.md")]
#![no_std]

/// Common between the Kernel and Userspace
pub use common;

/// The user must provide a `no_mangle` entrypoint.
extern "Rust" {
    fn entry() -> !;
}

#[link_section = ".anachro_table.entry_point"]
#[no_mangle]
#[used]
#[doc(hidden)]
pub static __ENTRY_POINT: unsafe fn() -> ! = entry;

use core::fmt::Write;
use core::panic::PanicInfo;

// Provide a basic panic handler. In the future, this will probably
// change to one or both of:
//
// * Being behind a feature, so you can provide your own panic handler
// * Attempt to print the panic to the stdout (e.g. serial port 0),
//     then triggering a "halt" or "reboot" system call.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut sop = StdOut;
    if let Some(location) = info.location() {
        writeln!(&mut sop, "Panicked at {}", location).ok();
    }
    common::porcelain::system::panic()
}

pub struct StdOut;

impl Write for StdOut {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        common::porcelain::serial::write_port(0, s.as_bytes()).ok();
        Ok(())
    }
}
