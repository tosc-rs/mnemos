#![no_main]
#![no_std]

use cassette::{pin_mut, Cassette};
use groundhog_nrf52::GlobalRollingTimer;
use pelle_bringup::{
    self as _, // global logger + panicking-behavior + memory layout
    map_pins,
    qspi::{QspiPins, Qspi, FlashChunk, EraseLength},
};
use nrf52840_hal::{pac::Peripherals, gpio::Level};
use byte_slab::{BSlab, ManagedArcSlab};

static SLAB: BSlab<4, 256> = BSlab::new();

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("Hello, world!");
    SLAB.init().ok();

    let board = defmt::unwrap!(Peripherals::take());
    let pins = map_pins(board.P0, board.P1);

    GlobalRollingTimer::init(board.TIMER0);

    let mut _led1 = pins.led1.into_push_pull_output(Level::Low);
    let mut _led2 = pins.led2.into_push_pull_output(Level::Low);

    let qpins = QspiPins {
        qspi_copi_io0: pins.qspi_d0.degrade(),
        qspi_cipo_io1: pins.qspi_d1.degrade(),
        qspi_io2: pins.qspi_d2.degrade(),
        qspi_io3: pins.qspi_d3.degrade(),
        qspi_csn: pins.qspi_csn.degrade(),
        qspi_sck: pins.qspi_sck.degrade(),
    };
    let mut qspi = Qspi::new(board.QSPI, qpins);

    let mut buf1 = [0xAC; 256];
    let mut buf2 = [0xAC; 256];

    // Read
    {
        let read_fut = qspi.read(0x0000_0000, &mut buf1);
        pin_mut!(read_fut);
        let mut read_cas = Cassette::new(read_fut);
        while read_cas.poll_on().is_none() { }
    }
    defmt::println!("{:?}", &buf1);

    defmt::println!("Erasing...");
    {
        let erase_fut = qspi.erase(0x0000_0000, EraseLength::_4KB);
        pin_mut!(erase_fut);
        let mut erase_cas = Cassette::new(erase_fut);
        while erase_cas.poll_on().is_none() { }
    }

    // Read
    {
        let read_fut = qspi.read(0x0000_0000, &mut buf2);
        pin_mut!(read_fut);
        let mut read_cas = Cassette::new(read_fut);
        while read_cas.poll_on().is_none() { }
    }
    defmt::println!("{:?}", &buf2);

    defmt::println!("Incrementing...");
    buf1.iter_mut().for_each(|b| {
        *b = b.wrapping_add(1);
    });

    {
        let write_fut = qspi.write(FlashChunk { addr: 0x0000_0000, data: ManagedArcSlab::<4, 256>::Borrowed(&mut buf1) });
        pin_mut!(write_fut);
        let mut write_cas = Cassette::new(write_fut);
        while write_cas.poll_on().is_none() { }
    }

    // Read
    {
        let read_fut = qspi.read(0x0000_0000, &mut buf2);
        pin_mut!(read_fut);
        let mut read_cas = Cassette::new(read_fut);
        while read_cas.poll_on().is_none() { }
    }
    defmt::println!("{:?}", &buf2);

    pelle_bringup::exit();
}
