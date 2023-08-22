use d1_pac::{Interrupt, GPIO};
use kernel::{embedded_hal, embedded_hal_async::digital::Wait, isr::Isr, maitake::sync::WaitCell};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Pin {
    /// Pin group PB
    B(PinB),
    /// Pin group PC
    C(PinC),
    /// Pin group PD
    D(PinD),
    /// Pin group PE
    E(PinE),
    /// Pin group PF
    F(PinF),
    /// Pin group PG
    G(PinG),
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinB {
    B0 = 0,
    B1,
    B2,
    B3,
    B4,
    B5,
    B6,
    B7,
    B8,
    B9,
    B10,
    B11,
    B12,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinC {
    C0 = 0,
    C1,
    C2,
    C3,
    C4,
    C5,
    C6,
    C7,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinD {
    D0 = 0,
    D1,
    D2,
    D3,
    D4,
    D5,
    D6,
    D7,
    D8,
    D9,
    D10,
    D11,
    D12,
    D13,
    D14,
    D15,
    D16,
    D17,
    D18,
    D19,
    D20,
    D21,
    D22,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinE {
    E0 = 0,
    E1,
    E2,
    E3,
    E4,
    E5,
    E6,
    E7,
    E8,
    E9,
    E10,
    E11,
    E12,
    E13,
    E14,
    E15,
    E16,
    E17,
    E18,
    E19,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinF {
    F0 = 0,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinG {
    G0 = 0,
    G1,
    G2,
    G3,
    G4,
    G5,
    G6,
    G7,
    G8,
    G9,
    G10,
    G11,
    G12,
    G13,
    G14,
    G15,
    G16,
    G17,
    G18,
    G19,
}

macro_rules! impl_from_pins {
    ($($P:ty => $pin:ident),+ $(,)?) => {
        $(
            impl From<$P> for Pin {
                fn from(p: $P) -> Self {
                    Self::$pin(p)
                }
            }
        )+
    }
}

impl_from_pins! {
    PinB => B,
    PinC => C,
    PinD => D,
    PinE => E,
    PinF => F,
    PinG => G,
}

impl embedded_hal::digital::ErrorType for PinB {
    type Error = core::convert::Infallible;
}

#[repr(u32)]
#[derive(Copy, Clone, Eq, PartialEq)]
enum Trigger {
    PosEdge = 0x0,
    NegEdge = 0x1,
    DoubleEdge = 0x4,
    HighLevel = 0x2,
    LowLevel = 0x3,
}

#[repr(u32)]
#[derive(Copy, Clone, Eq, PartialEq)]
enum PinMode {
    /// Input mode
    Input = 0b0000,
    /// Output mode
    Output = 0b0001,
    /// Alternate function 1 (pin-specific)
    Fn1 = 0b0010,
    /// Alternate function 2 (pin-specific)
    Fn2 = 0b0011,
    /// Alternate function 3 (pin-specific)
    Fn3 = 0b0100,
    /// Alternate function 4 (pin-specific)
    Fn4 = 0b0101,
    /// Input interrupt mode
    Irq = 0b1110,
    /// I/0 disabled.
    Off = 0b1111,
}

impl PinMode {
    /// Number of bits used for each pin's pin mode in the `{pin_block}_CFG{n}`
    /// registers.
    const BITS: u32 = Self::MASK.count_ones();
    const MASK: u32 = Self::Off as u32;

    #[inline(always)]
    fn set_bits(self, bits: u32, num: usize) -> u32 {
        let shift = num as u32 * Self::BITS;
        let mode = self as u32;
        let mask = !(Self::MASK) << shift;
        (bits & mask) | (mode << shift)
    }
}

impl Trigger {
    /// Number of bits used for each pin's IRQ trigger in the `{pin
    /// block}_EINT_CFG{n}` registers.
    const BITS: u32 = Self::MASK.count_ones();
    const MASK: u32 = 0b111;

    #[inline(always)]
    fn set_bits(self, bits: u32, num: usize) -> u32 {
        let shift = num as u32 * Self::BITS;
        let trigger = self as u32;
        let mask = !(Self::MASK) << shift;
        (bits & mask) | (trigger << shift)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct IrqPin<T> {
    pin: T,
}

impl IrqPin<PinB> {
    pub fn new(pin: PinB, gpio: &GPIO) -> Self {
        // TODO(eliza): this should probably assert nobody else is using that pin...
        let num = pin as u32;

        // disable the IRQ while writing to the register.
        gpio.pb_eint_ctl
            .modify(|r, w| unsafe { w.bits(r.bits() & !(1 << num)) });

        // first, make sure the pin is in interrupt mode.

        // the PB_CFG0 register has pins 0-7, while the PB_CFG1 register has
        // the remaining pins.
        if num >= 7 {
            let num = num - 7;
            gpio.pb_cfg1.modify(|r, w| {
                let bits = PinMode::Irq.set_bits(r.bits(), num as usize);
                unsafe { w.bits(bits) }
            })
        } else {
            gpio.pb_cfg0.modify(|r, w| {
                let bits = PinMode::Irq.set_bits(r.bits(), num as usize);
                unsafe { w.bits(bits) }
            })
        }

        Self { pin }
    }

    async fn wait_for_irq(&mut self, trigger: Trigger) {
        let num = self.pin as usize;

        let wait = PB_IRQS[num].subscribe().await;

        let gpio = unsafe { &*GPIO::ptr() };
        let int_enable = 1 << num;

        // disable the IRQ while writing to the register.
        gpio.pb_eint_ctl
            .modify(|r, w| unsafe { w.bits(r.bits() & !int_enable) });

        // set the trigger mode in either PB_EINT_CFG0 or PB_EINT_CFG1 depending
        // on the pin number.
        if num >= 7 {
            let num = num - 7;
            gpio.pb_eint_cfg1.modify(|r, w| {
                let bits = trigger.set_bits(r.bits(), num);
                unsafe { w.bits(bits) }
            });
        } else {
            gpio.pb_eint_cfg0.modify(|r, w| {
                let bits = trigger.set_bits(r.bits(), num);
                unsafe { w.bits(bits) }
            });
        }

        // enable the IRQ.
        gpio.pb_eint_ctl
            .modify(|r, w| unsafe { w.bits(r.bits() | int_enable) });

        wait.await.expect("waitcell should never be closed");

        // disable the IRQ again.
        gpio.pb_eint_ctl
            .modify(|r, w| unsafe { w.bits(r.bits() & !int_enable) });
    }
}

impl embedded_hal::digital::ErrorType for IrqPin<PinB> {
    type Error = core::convert::Infallible;
}

impl Wait for IrqPin<PinB> {
    async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
        self.wait_for_irq(Trigger::HighLevel).await;
        Ok(())
    }

    async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
        self.wait_for_irq(Trigger::LowLevel).await;
        Ok(())
    }

    async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
        self.wait_for_irq(Trigger::PosEdge).await;
        Ok(())
    }

    async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
        self.wait_for_irq(Trigger::NegEdge).await;
        Ok(())
    }

    async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
        self.wait_for_irq(Trigger::DoubleEdge).await;
        Ok(())
    }
}

pub(crate) const INTERRUPTS: [(Interrupt, fn()); 5] = [
    (Interrupt::GPIOB_NS, handle_pb_irq),
    (Interrupt::GPIOC_NS, handle_pc_irq),
    (Interrupt::GPIOD_NS, handle_pd_irq),
    (Interrupt::GPIOE_NS, handle_pe_irq),
    (Interrupt::GPIOF_NS, handle_pf_irq),
    // is there not an interrupt for GPIO pin group G? the manual says
    // those pins can also have interrupts, but there's no `Interrupt` variant
    // in `d1_pac`...
    // (Interrupt::GPIOG_NS, handle_pg_irq)
];

#[allow(clippy::declare_interior_mutable_const)]
const NEW_WAITCELL: WaitCell = WaitCell::new();
static PB_IRQS: [WaitCell; PB_COUNT] = [NEW_WAITCELL; PB_COUNT];
static PC_IRQS: [WaitCell; PC_COUNT] = [NEW_WAITCELL; PC_COUNT];
static PD_IRQS: [WaitCell; PD_COUNT] = [NEW_WAITCELL; PD_COUNT];
static PE_IRQS: [WaitCell; PE_COUNT] = [NEW_WAITCELL; PE_COUNT];
static PF_IRQS: [WaitCell; PF_COUNT] = [NEW_WAITCELL; PF_COUNT];
static PG_IRQS: [WaitCell; PG_COUNT] = [NEW_WAITCELL; PG_COUNT];

const PB_COUNT: usize = 13;
const PC_COUNT: usize = 8;
const PD_COUNT: usize = 23;
const PE_COUNT: usize = 18;
const PF_COUNT: usize = 7;
const PG_COUNT: usize = 19;

macro_rules! isrs {
    ($($vis:vis fn $name:ident($register:ident, $waiters:ident);)+) => {
        $(
            $vis fn $name() {
                debug_assert!(Isr::is_in_isr());
                let gpio = unsafe { &*GPIO::ptr() };
                gpio.$register.modify(|r, w| {
                    tracing::trace!($register = ?format_args!("{:#b}", r.bits()), "GPIO interrupt");
                    for (bit, waiters) in $waiters.iter().enumerate() {
                        let bit = unsafe {
                            // Safety: the length of each IRQ waker array is the
                            // same length as the register.
                            r.eint_status(bit as u8)
                        };
                        if bit.is_pending() {
                            waiters.wake();
                        }
                    }
                    // write back any set bits to clear those IRQs.
                    w
                })
            }
        )+

    }
}

isrs! {
    pub(crate) fn handle_pb_irq(pb_eint_status, PB_IRQS);
    pub(crate) fn handle_pc_irq(pc_eint_status, PC_IRQS);
    pub(crate) fn handle_pd_irq(pd_eint_status, PD_IRQS);
    pub(crate) fn handle_pe_irq(pe_eint_status, PE_IRQS);
    pub(crate) fn handle_pf_irq(pf_eint_status, PF_IRQS);
    // pub(crate) fn handle_pg_irq(pg_eint_status, PG_IRQS);
}

// struct IrqLock<T> {
//     data: UnsafeCell<T>,
// }

// struct IrqGuard<'a, T> {
//     data: &'a mut T,
// }

// unsafe impl<T: Sync> Sync for IrqLock<T> {}

// impl<T> IrqLock<T> {
//     unsafe fn get_irq(&self) -> *mut T {
//         debug_assert!(Isr::is_in_isr());
//         self.data.get()
//     }

//     fn lock(&self) -> IrqGuard<'_, T> {
//         unsafe {
//             riscv::interrupt::disable();
//             IrqGuard {
//                 data: &mut *self.data.get(),
//             }
//         }
//     }
// }

// impl<T> Deref for IrqGuard<'_, T> {
//     type Target = T;
//     fn deref(&self) -> &Self::Target {
//         self.data
//     }
// }

// impl<T> DerefMut for IrqGuard<'_, T> {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         self.data
//     }
// }

// impl<T> Drop for IrqGuard<'_, T> {
//     fn drop(&mut self) {
//         unsafe {
//             riscv::interrupt::enable();
//         }
//     }
// }
