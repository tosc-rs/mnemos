#![no_std]
#![no_main]

use mstd as _; // Panic handler

// I'm just here so the program can link.
#[no_mangle]
fn entry() -> ! {
    panic!();
}
