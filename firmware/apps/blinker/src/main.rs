#![no_std]
#![no_main]

use userspace::common::{porcelain::gpio, syscall::request::GpioMode::OutputPushPull};
use userspace::common::porcelain::spi;

#[no_mangle]
pub fn entry() -> ! {
    gpio::set_mode(3, OutputPushPull { is_high: false }).ok();
    let mut send_buf = [0u8; 8];
    send_buf.copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);

    loop {
        gpio::write_output(3, true).ok();
        gpio::write_output(3, false).ok();
        for i in 0..5 {
            spi::send(i, &send_buf, 32_000).ok();
        }
    }
}
