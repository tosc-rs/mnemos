#![no_std]

pub use common;

// The user must provide a `no_mangle` entrypoint.
extern "Rust" {
    fn entry() -> !;
}

#[link_section = ".anachro_table.entry_point"]
#[no_mangle]
#[used]
pub static __ENTRY_POINT: unsafe fn() -> ! = entry;

use core::panic::PanicInfo;
use core::sync::atomic::{self, Ordering};

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        atomic::compiler_fence(Ordering::SeqCst);
    }
}
