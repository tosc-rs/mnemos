#![no_std]
#![no_main]

use userspace::common::{porcelain::gpio, syscall::request::GpioMode::OutputPushPull};

#[no_mangle]
pub fn entry() -> ! {
    gpio::set_mode(3, OutputPushPull { is_high: false }).ok();

    loop {
        gpio::write_output(3, true).ok();
        gpio::write_output(3, false).ok();
    }
}
