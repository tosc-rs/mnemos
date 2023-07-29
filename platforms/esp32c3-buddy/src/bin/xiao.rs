#![no_std]
#![no_main]

extern crate alloc;

use esp32c3_hal::{
    clock::ClockControl, peripherals::Peripherals, prelude::*, systimer::SystemTimer,
    timer::TimerGroup, Rtc, IO,
};
use esp_backtrace as _;
use esp_println::println;
use mnemos_esp32c3_buddy::drivers;

#[entry]
fn main() -> ! {
    unsafe {
        mnemos_esp32c3_buddy::heap::init();
    }

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
    println!("Hello world!");

    let k = mnemos_esp32c3_buddy::init();

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);

    // initialize SimpleSerial driver
    k.initialize({
        use esp32c3_hal::uart::{
            config::{Config, DataBits, Parity, StopBits},
            TxRxPins, Uart,
        };

        let config = Config {
            baudrate: 115200,
            data_bits: DataBits::DataBits8,
            parity: Parity::ParityNone,
            stop_bits: StopBits::STOP1,
        };

        let pins = TxRxPins::new_tx_rx(
            io.pins.gpio1.into_push_pull_output(),
            io.pins.gpio2.into_floating_input(),
        );

        let uart0 = Uart::new_with_config(
            peripherals.UART0,
            Some(config),
            Some(pins),
            &clocks,
            &mut system.peripheral_clock_control,
        );

        drivers::uart::C3Uart::uart0(uart0).register(k, 4096, 4096)
    })
    .unwrap();

    mnemos_esp32c3_buddy::spawn_daemons(k);

    // configure system timer
    let syst = SystemTimer::new(peripherals.SYSTIMER);

    println!("SYSTIMER Current value = {}", SystemTimer::now());

    // Alarm 1 will be used to generate "sleep until" interrupts.
    let alarm1 = syst.alarm1;

    mnemos_esp32c3_buddy::run(&k, alarm1)
}
