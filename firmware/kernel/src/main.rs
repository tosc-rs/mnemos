#![no_main]
#![no_std]

use kernel as _; // global logger + panicking-behavior + memory layout

#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
    use cortex_m::singleton;
    use defmt::unwrap;
    // use heapless::spsc::Queue;
    use nrf52840_hal::{
        clocks::{ExternalOscillator, Internal, LfOscStopped},
    //     gpio::{p0::Parts as P0Parts, p1::Parts as P1Parts, Level},
        pac::TIMER0,
        // pac::{SPIM2, TIMER0, TWIM0},
    //     spim::{Frequency as SpimFreq, Pins as SpimPins, Spim, MODE_0},
    //     twim::{Frequency as TwimFreq, Pins as TwimPins, Twim},
    //     uarte::{Baudrate, Parity, Pins as UartPins, Uarte},
    //     usbd::{UsbPeripheral, Usbd},
        Clocks,
    };
    use kernel::monotonic::{ExtU32, MonoTimer};
    // use nrf52_phm::uart::PhmUart;
    // use phm_icd::{ToMcu, ToPc};
    // use phm_worker::{
    //     comms::{CommsLink, InterfaceComms, WorkerComms},
    //     Worker,
    // };
    // use postcard::{to_vec_cobs, CobsAccumulator, FeedResult};
    // use usb_device::{
    //     class_prelude::UsbBusAllocator,
    //     device::{UsbDevice, UsbDeviceBuilder, UsbVidPid},
    // };
    // use usbd_serial::{SerialPort, USB_CLASS_CDC};

    #[monotonic(binds = TIMER0, default = true)]
    type Monotonic = MonoTimer<TIMER0>;

    #[shared]
    struct Shared {}

    #[local]
    struct Local {
        lol: ()
    }

    #[init(local = [])]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        let device = cx.device;

        // Setup clocks early in the process. We need this for USB later
        let clocks = Clocks::new(device.CLOCK);
        let clocks = clocks.enable_ext_hfosc();
        let clocks =
            unwrap!(singleton!(: Clocks<ExternalOscillator, Internal, LfOscStopped> = clocks));

        // Configure the monotonic timer, currently using TIMER0, a 32-bit, 1MHz timer
        let mono = Monotonic::new(device.TIMER0);

        (
            Shared {},
            Local {
                lol: (),
            },
            init::Monotonics(mono),
        )
    }

    // #[task(local = [usb_serial, interface_comms, usb_dev, cobs_buf: CobsAccumulator<512> = CobsAccumulator::new()])]
    // fn usb_tick(cx: usb_tick::Context) {
    //     let usb_serial = cx.local.usb_serial;
    //     let usb_dev = cx.local.usb_dev;
    //     let cobs_buf = cx.local.cobs_buf;
    //     let interface_comms = cx.local.interface_comms;

    //     let mut buf = [0u8; 128];

    //     usb_dev.poll(&mut [usb_serial]);

    //     if let Some(out) = interface_comms.to_pc.dequeue() {
    //         if let Ok(ser_msg) = to_vec_cobs::<_, 128>(&out) {
    //             usb_serial.write(&ser_msg).ok();
    //         } else {
    //             defmt::panic!("Serialization error!");
    //         }
    //     }

    //     match usb_serial.read(&mut buf) {
    //         Ok(sz) if sz > 0 => {
    //             let buf = &buf[..sz];
    //             let mut window = &buf[..];

    //             'cobs: while !window.is_empty() {
    //                 window = match cobs_buf.feed::<phm_icd::ToMcu>(&window) {
    //                     FeedResult::Consumed => break 'cobs,
    //                     FeedResult::OverFull(new_wind) => new_wind,
    //                     FeedResult::DeserError(new_wind) => new_wind,
    //                     FeedResult::Success { data, remaining } => {
    //                         defmt::println!("got: {:?}", data);
    //                         interface_comms.to_mcu.enqueue(data).ok();
    //                         remaining
    //                     }
    //                 };
    //             }
    //         }
    //         Ok(_) | Err(usb_device::UsbError::WouldBlock) => {}
    //         Err(_e) => defmt::panic!("Usb Error!"),
    //     }

    //     usb_tick::spawn_after(1.millis()).ok();
    // }

    #[idle(local = [])]
    fn idle(cx: idle::Context) -> ! {
        defmt::println!("Hello, world!");
        // let worker = cx.local.worker;

        loop {
            // unwrap!(worker.step().map_err(drop));
        }
    }
}
