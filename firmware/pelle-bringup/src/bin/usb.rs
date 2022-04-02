#![no_main]
#![no_std]

use cortex_m::singleton;
use groundhog_nrf52::GlobalRollingTimer;
use pelle_bringup::{
    self as _, // global logger + panicking-behavior + memory layout
};
use nrf52840_hal::{pac::Peripherals, clocks::{Clocks, ExternalOscillator, Internal, LfOscStopped}, usbd::{Usbd, UsbPeripheral}};
use usb_device::{class_prelude::UsbBusAllocator, device::{UsbVidPid, UsbDeviceBuilder}};
use usbd_serial::{SerialPort, USB_CLASS_CDC, UsbError};


#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("Hello, world!");

    let board = defmt::unwrap!(Peripherals::take());

    // Setup clocks early in the process. We need this for USB later
    let clocks: &'static mut _ = {
        let clocks = Clocks::new(board.CLOCK);
        let clocks = clocks.enable_ext_hfosc();
        defmt::unwrap!(cortex_m::singleton!(: Clocks<ExternalOscillator, Internal, LfOscStopped> = clocks))
    };

    GlobalRollingTimer::init(board.TIMER0);

    let (mut usb_dev, mut usb_serial) = {
        let usb_bus = Usbd::new(UsbPeripheral::new(board.USBD, clocks));
        let usb_bus = defmt::unwrap!(singleton!(:UsbBusAllocator<Usbd<UsbPeripheral>> = usb_bus));

        let usb_serial = SerialPort::new(usb_bus);
        let usb_dev = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
            .manufacturer("OVAR Labs")
            .product("Anachro Pellegrino")
            // TODO: Use some kind of unique ID. This will probably require another singleton,
            // as the storage must be static. Probably heapless::String -> singleton!()
            .serial_number("ajm001")
            .device_class(USB_CLASS_CDC)
            .max_packet_size_0(64) // (makes control transfers 8x faster)
            .build();

        (usb_dev, usb_serial)
    };

    let mut buf = [0u8; 128];

    loop {
        if usb_dev.poll(&mut [&mut usb_serial]) {
            match usb_serial.read(&mut buf) {
                Ok(sz) if sz > 0 => {
                    usb_serial.write(&buf[..sz]).ok();
                },
                Ok(_) | Err(UsbError::WouldBlock) => continue,
                Err(_e) => defmt::panic!("Usb Error!"),
            };
        }
    }
}
