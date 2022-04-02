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

    let mut acc = Accumulator::<1024>::new();
    let mut dec_buf = [0u8; 1024];

    loop {
        if usb_dev.poll(&mut [&mut usb_serial]) {
            match usb_serial.read(&mut buf) {
                Ok(sz) if sz > 0 => {
                    let mut window = &buf[..sz];

                    while !window.is_empty() {
                        match acc.feed(window) {
                            Ok(Some(msg)) => {
                                match sportty::Message::decode_to(msg.msg.as_slice(), &mut dec_buf) {
                                    Ok(smsg) => {
                                        defmt::println!("[{=u16:05}]: {=[u8]}", smsg.port, smsg.data);
                                    },
                                    Err(_) => defmt::println!("Sportty error!"),
                                }
                                window = msg.remainder;
                            },
                            Ok(None) => {},
                            Err(_) => {
                                defmt::println!("Decode error!");
                            },
                        }
                    }
                },
                Ok(_) | Err(UsbError::WouldBlock) => continue,
                Err(_e) => defmt::panic!("Usb Error!"),
            };
        }
    }
}

struct Accumulator<const N: usize> {
    buf: [u8; N],
    idx: usize,
}

enum AccError<'a> {
    NoRoomNoRem,
    NoRoomWithRem(&'a [u8]),
}

impl<const N: usize> Accumulator<N> {
    fn new() -> Self {
        Self {
            buf: [0u8; N],
            idx: 0,
        }
    }
    fn feed<'a>(&mut self, buf: &'a [u8]) -> Result<Option<AccSuccess<'a, N>>, AccError<'a>> {
        match buf.iter().position(|b| *b == 0) {
            Some(n) if (self.idx + n) <= N => {
                let (now, later) = buf.split_at(n + 1);
                self.buf[self.idx..][..now.len()].copy_from_slice(now);
                let mut msg = AccMsg {
                    buf: [0u8; N],
                    len: self.idx + now.len(),
                };
                msg.buf[..msg.len].copy_from_slice(&self.buf[..msg.len]);
                self.idx = 0;
                Ok(Some(AccSuccess {
                    remainder: later,
                    msg,
                }))
            },
            Some(n) if n < buf.len() => {
                self.idx = 0;
                Err(AccError::NoRoomWithRem(&buf[(n + 1)..]))
            },
            Some(_) => {
                self.idx = 0;
                Err(AccError::NoRoomNoRem)
            }
            None if (self.idx + buf.len()) <= N => {
                self.buf[self.idx..][..buf.len()].copy_from_slice(buf);
                self.idx += buf.len();
                Ok(None)
            },
            None => {
                // No room, and no zero. Truncate the current buf.
                self.idx = 0;
                Err(AccError::NoRoomNoRem)
            },
        }
    }
}

struct AccSuccess<'a, const N: usize> {
    remainder: &'a [u8],
    msg: AccMsg<N>,
}

struct AccMsg<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> AccMsg<N> {
    fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}
