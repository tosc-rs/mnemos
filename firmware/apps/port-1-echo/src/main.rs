#![no_std]
#![no_main]

use userspace::common::porcelain::{serial, time};

#[no_mangle]
pub fn entry() -> ! {
    // First, open Port 1 (we will write to it)
    serial::open_port(1).unwrap();

    let mut buf = [0u8; 128];

    loop {
        if let Ok(data) = serial::read_port(0, &mut buf) {
            serial::write_port(1, data).ok();
        }

        time::sleep_micros(10_000).ok();
    }
}
