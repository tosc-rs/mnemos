#![no_std]
#![no_main]

extern crate alloc;

use esp32c3_hal::{
    clock::ClockControl, peripherals::Peripherals, prelude::*, systimer::SystemTimer,
    timer::TimerGroup, Rtc,
};
use esp_backtrace as _;

#[entry]
fn main() -> ! {
    mnemos_esp32c3_buddy::heap::init();

    let peripherals = Peripherals::take();
    let mut system = peripherals.SYSTEM.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    // Disable the RTC and TIMG watchdog timers
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt1 = timer_group1.wdt;
    rtc.swd.disable();
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();

    let k = mnemos_esp32c3_buddy::init();
    mnemos_esp32c3_buddy::spawn_serial(
        k,
        peripherals.USB_DEVICE,
        &mut system.peripheral_clock_control,
    );
    mnemos_esp32c3_buddy::spawn_daemons(k);

    // configure system timer
    let syst = SystemTimer::new(peripherals.SYSTIMER);
    // Alarm 1 will be used to generate "sleep until" interrupts.
    let alarm1 = syst.alarm1;

    mnemos_esp32c3_buddy::run(k, alarm1)
}
