#![no_std]
#![no_main]

use userspace as _; // Panic handler

#[no_mangle]
fn entry() -> ! {
    panic!();
}
