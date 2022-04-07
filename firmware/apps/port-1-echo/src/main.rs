#![no_std]
#![no_main]

use userspace::common::porcelain::{serial, time};

#[no_mangle]
pub fn entry() -> ! {
    // First, open Port 1 (we will write to it)
    serial::open_port(1).unwrap();

    let mut buf = [0u8; 128];

    loop {
        for _ in 0..100 {
            if let Ok(data) = serial::read_port(0, &mut buf) {
                match serial::write_port(1, data) {
                    Ok(None) => {},
                    Ok(Some(_)) => {},
                    Err(()) => {},
                }
            } else {
                // defmt::println!("Read port 0 failed!");
            }

            time::sleep_micros(10_000).ok();
        }

        // if let Some(hg) = HEAP.try_lock() {
        //     let used = hg.used_space();
        //     let free = hg.free_space();
        //     defmt::println!("used: {=usize}, free: {=usize}", used, free);
        // }
    }
}
