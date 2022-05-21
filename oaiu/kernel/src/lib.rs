#![no_main]
#![no_std]

use core::{cell::UnsafeCell, mem::MaybeUninit};

use defmt_rtt as _; // global logger

use groundhog::RollingTimer;
use groundhog_nrf52::GlobalRollingTimer;
use nrf52840_hal::{
    self,
    gpio::{
        p0::{
            P0_02, P0_03, P0_04, P0_05, P0_06, P0_07, P0_08, P0_09, P0_10, P0_11, P0_12, P0_13,
            P0_14, P0_15, P0_16, P0_17, P0_18, P0_19, P0_20, P0_21, P0_22, P0_23, P0_24, P0_25,
            P0_26, P0_27, P0_28, P0_29, P0_30, P0_31,
        },
        p1::{P1_00, P1_02, P1_08, P1_09, P1_10, P1_15},
        Disconnected,
    },
    pac::{P0, P1},
}; // memory layout
use heapless::mpmc::MpMcQueue;

use panic_probe as _;
pub mod alloc;
pub mod drivers;
pub mod future_box;
pub mod monotonic;
pub mod traits;
pub mod syscall;

// same panicking *behavior* as `panic-probe` but doesn't print a panic message
// this prevents the panic message being printed *twice* when `defmt::panic` is invoked
#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}

defmt::timestamp!("{=u32:us}", {
    GlobalRollingTimer::default().get_ticks() as u32
});

/// Terminates the application and makes `probe-run` exit with exit-code = 0
pub fn exit() -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}

pub struct Pins {
    /// HS
    pub a00: P0_04<Disconnected>,
    /// HS
    pub a01: P0_05<Disconnected>,
    /// LS
    pub a02: P0_30<Disconnected>,
    /// LS
    pub a03: P0_28<Disconnected>,
    /// LS
    pub a04: P0_02<Disconnected>,
    /// LS
    pub a05: P0_03<Disconnected>,
    /// LS
    /// NOTE: 0.1uF cap to ground
    pub aref: P0_31<Disconnected>,
    /// Connected to VDIV, 100K/100K (50%) Resistor divider
    /// from the 'VBAT' line
    pub vdiv: P0_29<Disconnected>,

    // LS - NFC pin, limited functionality
    pub d02: P0_10<Disconnected>,
    /// HS
    pub d05: P1_08<Disconnected>,
    /// HS
    pub d06: P0_07<Disconnected>,
    /// HS
    pub d09: P0_26<Disconnected>,
    /// HS
    pub d10: P0_27<Disconnected>,
    /// HS
    pub d11: P0_06<Disconnected>,
    /// HS
    pub d12: P0_08<Disconnected>,
    /// HS
    pub d13: P1_09<Disconnected>,

    /// HS
    pub scl: P0_11<Disconnected>,
    /// HS
    pub sda: P0_12<Disconnected>,
    /// HS
    pub rxd: P0_24<Disconnected>,
    /// HS
    pub txd: P0_25<Disconnected>,
    /// HS
    pub sclk: P0_14<Disconnected>,
    /// HS
    pub mosi: P0_13<Disconnected>,
    /// HS
    pub miso: P0_15<Disconnected>,

    // QSPI Pins
    // NOTE: Flash chip is a GD25Q16, a 2MiB flash
    pub qspi_d0: P0_17<Disconnected>,
    pub qspi_d1: P0_22<Disconnected>,
    pub qspi_d2: P0_23<Disconnected>,
    pub qspi_d3: P0_21<Disconnected>,
    pub qspi_sck: P0_19<Disconnected>,
    pub qspi_csn: P0_20<Disconnected>,

    /// Red, Active High
    pub led1: P1_15<Disconnected>,
    /// Blue, Active High
    pub led2: P1_10<Disconnected>,
    /// neopixel
    pub neopix: P0_16<Disconnected>,
    /// active low, needs internal pullup (if used by sw)
    pub reset: P0_18<Disconnected>,
    /// active low, needs internal pullup
    pub switch: P1_02<Disconnected>,
    /// gpio - on debug connector
    pub swo: P1_00<Disconnected>,
    /// gpio - NFC pin, limited functionality
    pub tp1: P0_09<Disconnected>,
}

use nrf52840_hal::gpio::p0::Parts as P0Parts;
use nrf52840_hal::gpio::p1::Parts as P1Parts;

pub fn map_pins(p0: P0, p1: P1) -> Pins {
    let p0 = P0Parts::new(p0);
    let p1 = P1Parts::new(p1);

    Pins {
        a00: p0.p0_04,
        a01: p0.p0_05,
        a02: p0.p0_30,
        a03: p0.p0_28,
        a04: p0.p0_02,
        a05: p0.p0_03,
        aref: p0.p0_31,
        vdiv: p0.p0_29,
        d02: p0.p0_10,
        d05: p1.p1_08,
        d06: p0.p0_07,
        d09: p0.p0_26,
        d10: p0.p0_27,
        d11: p0.p0_06,
        d12: p0.p0_08,
        d13: p1.p1_09,
        scl: p0.p0_11,
        sda: p0.p0_12,
        rxd: p0.p0_24,
        txd: p0.p0_25,
        sclk: p0.p0_14,
        mosi: p0.p0_13,
        miso: p0.p0_15,
        qspi_d0: p0.p0_17,
        qspi_d1: p0.p0_22,
        qspi_d2: p0.p0_23,
        qspi_d3: p0.p0_21,
        qspi_sck: p0.p0_19,
        qspi_csn: p0.p0_20,
        led1: p1.p1_15,
        led2: p1.p1_10,
        neopix: p0.p0_16,
        reset: p0.p0_18,
        switch: p1.p1_02,
        swo: p1.p1_00,
        tp1: p0.p0_09,
    }
}

#[link_section = ".uninit.magic_boot"]
pub static MAGIC_BOOT: MagicBoot = MagicBoot {
    tag: UnsafeCell::new(MaybeUninit::uninit()),
};

pub struct MagicBoot {
    tag: UnsafeCell<MaybeUninit<u32>>,
}

impl MagicBoot {
    const UPPER_MAGIC: u32 = 0xB007_6000;
    const MAGIC_MASK: u32 = 0xFFFF_FF00;

    pub fn read_clear(&self) -> Option<u32> {
        let tag = unsafe {
            let val = self.tag.get().read_volatile();
            self.tag.get().write_volatile(MaybeUninit::new(0));
            val.assume_init()
        };

        if (tag & Self::MAGIC_MASK) == Self::UPPER_MAGIC {
            Some(tag & !Self::MAGIC_MASK)
        } else {
            None
        }
    }

    pub fn set(&self, block: u32) {
        if block > 255 {
            return;
        }
        let val = Self::UPPER_MAGIC | (block & !Self::MAGIC_MASK);
        unsafe {
            self.tag.get().write_volatile(MaybeUninit::new(val));
        }
    }
}

unsafe impl Sync for MagicBoot {}

pub enum DriverCommand {
    SpiStart,
    SpiEnd,
    SleepMicros(u32),
}

pub static DRIVER_QUEUE: MpMcQueue<DriverCommand, 64> = MpMcQueue::new();
