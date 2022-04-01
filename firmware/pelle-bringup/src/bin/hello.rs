#![no_main]
#![no_std]

use pelle_bringup as _; // global logger + panicking-behavior + memory layout

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("Hello, world!");

    pelle_bringup::exit()
}
