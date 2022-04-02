#![no_main]
#![no_std]

use groundhog_nrf52::GlobalRollingTimer;
use pelle_bringup::{
    self as _, // global logger + panicking-behavior + memory layout
    map_pins,
};
use nrf52840_hal::{pac::Peripherals, gpio::Level, prelude::OutputPin};
use smart_leds::{self, colors, gamma, brightness};
use smart_leds_trait::SmartLedsWrite;
use nrf_smartled::pwm::Pwm;
use groundhog::RollingTimer;

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("Hello, world!");
    let board = defmt::unwrap!(Peripherals::take());
    let pins = map_pins(board.P0, board.P1);

    let mut neopixel = Pwm::new(board.PWM0, pins.neopix.degrade());
    GlobalRollingTimer::init(board.TIMER0);
    let timer = GlobalRollingTimer::new();

    let colors = [
        colors::RED,
        colors::ORANGE,
        colors::YELLOW,
        colors::GREEN,
        colors::BLUE,
        colors::INDIGO,
        colors::VIOLET,
        colors::BLACK,
    ];

    let mut led1 = pins.led1.into_push_pull_output(Level::Low);
    let mut led2 = pins.led2.into_push_pull_output(Level::Low);
    let mut swap = false;

    loop {
        for color in colors.into_iter() {
            if swap {
                led1.set_low().ok();
                led2.set_high().ok();
            } else {
                led1.set_high().ok();
                led2.set_low().ok();
            }
            swap = !swap;

            neopixel.write(brightness(gamma([color].into_iter()), 16)).ok();
            let start = timer.get_ticks();
            while timer.millis_since(start) <= 333 { }
        }
    }
}
