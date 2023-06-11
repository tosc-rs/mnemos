#![no_std]
#![no_main]

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    loop {

    }
}

use core::panic::PanicInfo;

#[panic_handler]
fn handler(_info: &PanicInfo) -> ! {
    loop {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}
