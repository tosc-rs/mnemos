#![doc = include_str!("../README.md")]
#![no_std]

/// Common between the Kernel and Userspace
pub use abi;

pub mod executor;
pub mod serial;
pub mod utils;

// The user must provide a `no_mangle` entrypoint.
extern "Rust" {
    fn entry() -> !;
}

#[link_section = ".anachro_table.entry_point"]
#[no_mangle]
#[used]
#[doc(hidden)]
pub static __ENTRY_POINT: unsafe fn() -> ! = entry;

// Provide a basic panic handler. In the future, this will probably
// change to one or both of:
//
// * Being behind a feature, so you can provide your own panic handler
// * Attempt to print the panic to the stdout (e.g. serial port 0),
//     then triggering a "halt" or "reboot" system call.
#[cfg(feature = "panic-handler")]
mod panic_handler {
    use core::panic::PanicInfo;
    use core::sync::atomic::{compiler_fence, Ordering};

    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        // let mut sop = StdOut;
        // if let Some(location) = info.location() {
        //     writeln!(&mut sop, "Panicked at {}", location).ok();
        // }
        // abi::porcelain::system::panic()
        loop {
            compiler_fence(Ordering::SeqCst);
        }
    }
}
