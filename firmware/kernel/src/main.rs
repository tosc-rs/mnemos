#![no_main]
#![no_std]

use kernel as _; // global logger + panicking-behavior + memory layout

// use tasks::usb::UsbResources;

// use common::{
//     board::Mode,
//     usb_icd::{UsbFromHost, UsbToHost},
// };
// use uarte_485::Uarte485;

// use groundhog_nrf52::GlobalRollingTimer;
// use nrf52840_hal::{
//     gpio::{p0, Disconnected},
//     pac::{PWM0, PWM1, TIMER1, UARTE0, NVMC, SPIM3},
//     ppi::Ppi3,
//     saadc::Saadc,
//     Spim,
// };
// use nrf_smartled::pwm::Pwm;
// use heapless::spsc::{Consumer, Producer};

// use anachro_485::icd::{SLAB_SIZE, TOTAL_SLABS};
// use anachro_qspi::Qspi;
// use rand_chacha::ChaCha8Rng;

// pub mod tasks;

#[rtic::app(
    device = nrf52840_hal::pac,
    peripherals = true,
    monotonic = groundhog_nrf52::GlobalRollingTimer
)]
const APP: () = {
    struct Resources {
        lol: ()
    }

    #[init]
    fn init(cx: init::Context) -> init::LateResources {
        init::LateResources {
            lol: ()
        }
    }

    #[idle]
    fn idle(cx: idle::Context) -> ! {
        loop {

        }
    }

    // Sacrificial hardware interrupts
    // extern "C" {
    //     fn SWI0_EGU0();
    // }
};
