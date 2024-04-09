// TODO: add docs to these methods...
#![allow(clippy::missing_safety_doc)]

use core::{
    ptr::null_mut,
    sync::atomic::{AtomicPtr, Ordering},
};

use d1_pac::{plic, Interrupt, PLIC};
use kernel::isr::Isr;

/// Interrupt Priority from 0..31
pub type Priority = plic::prio::PRIORITY_A;

#[doc = r" TryFromPrioritytError"]
#[derive(Debug, Copy, Clone)]
pub struct TryFromPriorityError(());

/// Errors returned by [`Plic::activate`] and [`Plic::deactivate`].
#[derive(Debug, Copy, Clone)]
pub enum MaskError {
    /// No such interrupt was found!
    NotFound(Interrupt),
    /// The interrupt did not have a handler.
    NoHandler(Interrupt),
}

/// Platform-Level Interrupt Controller (PLIC) interface
pub struct Plic {
    plic: PLIC,
}

impl Plic {
    /// Create a new `Plic` from the [`PLIC`] peripheral
    pub fn new(plic: PLIC) -> Self {
        // TODO any initial setup we should be doing for the PLIC at startup?
        Self { plic }
    }

    /// Obtain a static `Plic` instance for use in e.g. interrupt handlers
    ///
    /// # Safety
    ///
    /// 'Tis thine responsibility, that which thou doth summon.
    pub unsafe fn summon() -> Self {
        Self {
            plic: d1_pac::Peripherals::steal().PLIC,
        }
    }

    /// Enable an interrupt
    ///
    /// # Safety
    ///
    /// May effect normal interrupt processing
    pub unsafe fn unmask(&self, interrupt: Interrupt) {
        let (mie, irq_en) = self.index_mie(interrupt);
        mie.modify(|r, w| w.bits(r.bits() | irq_en));
    }

    /// Disable an interrupt
    pub fn mask(&self, interrupt: Interrupt) {
        let (mie, irq_en) = self.index_mie(interrupt);
        mie.modify(|r, w| unsafe { w.bits(r.bits() | irq_en) });
    }

    /// Globally set priority for one interrupt
    ///
    /// # Safety
    ///
    /// May effect normal interrupt processing
    pub unsafe fn set_priority(&self, interrupt: Interrupt, priority: Priority) {
        let nr = interrupt.into_bits() as usize;
        self.plic.prio[nr].write(|w| w.bits(priority.into_bits()));
    }

    pub fn claim(&self) -> Interrupt {
        let claim = self.plic.mclaim.read().mclaim().bits() as u8;
        match Interrupt::try_from(claim) {
            Ok(interrupt) => interrupt,
            Err(e) => {
                panic!("error claiming interrupt: {e:?}");
            }
        }
    }

    /// Dispatch an interrupt to a vectored handler.
    ///
    /// # Safety
    ///
    /// Should only be called in an ISR such as `MachineInternal`!
    pub unsafe fn dispatch_interrupt(&self) {
        debug_assert!(
            Isr::is_in_isr(),
            "Plic::dispatch should only be called in an ISR!"
        );
        let claim = self.claim();
        let claim_u16 = claim as u16;

        // Is this a known interrupt?
        // N.B. that the index is always in bounds, because `claim()` has
        // panicked already if the interrupt is not a valid interrupt number.
        // Theoretically, we could probably optimize this to use
        // `get_unchecked`, if we cared to.
        let &Vectored { id, ref handler } = &INTERRUPT_ARRAY[claim_u16 as usize];
        debug_assert_eq!(
            id,
            Some(claim),
            "FLAGRANT ERROR: interrupt ID {id:?} does not match index \
                ({claim_u16}); perhaps the interrupt dispatch table has \
                somehow been corrupted?"
        );
        let ptr = handler.load(Ordering::SeqCst); // todo: ordering
        if !ptr.is_null() {
            let hdlr: fn() = unsafe { core::mem::transmute(ptr) };
            (hdlr)();
        } // otherwise, the ISR hasn't been registered yet; just do nothing.

        // Release claim
        self.complete(claim);
    }

    pub fn complete(&self, interrupt: Interrupt) {
        self.plic
            .mclaim
            .write(|w| w.mclaim().variant(interrupt.into_bits() as u16));
    }

    #[track_caller]
    pub unsafe fn register(&self, interrupt: Interrupt, new_hdl: fn()) {
        let idx = interrupt as usize;
        let Some(&Vectored { id, ref handler }) = INTERRUPT_ARRAY.get(idx) else {
            panic!("interrupt not found in dispatch table: {interrupt:?} (index {idx})")
        };
        assert_eq!(
            Some(interrupt),
            id,
            "FLAGRANT ERROR: interrupt ID for {interrupt:?} does not \
            match index {idx}; perhaps the interrupt dispatch table has \
            somehow been corrupted?"
        );
        handler.store(new_hdl as *mut fn() as *mut (), Ordering::Release);
    }

    pub unsafe fn activate(&self, interrupt: Interrupt, prio: Priority) -> Result<(), MaskError> {
        self.can_mask(interrupt)?;
        self.set_priority(interrupt, prio);
        self.unmask(interrupt);
        Ok(())
    }

    pub fn deactivate(&self, interrupt: Interrupt) -> Result<(), MaskError> {
        self.can_mask(interrupt)?;
        self.mask(interrupt);
        Ok(())
    }

    fn can_mask(&self, interrupt: Interrupt) -> Result<(), MaskError> {
        let &Vectored { id, ref handler } = INTERRUPT_ARRAY
            .get(interrupt as usize)
            .ok_or(MaskError::NotFound(interrupt))?;

        if id != Some(interrupt) {
            return Err(MaskError::NotFound(interrupt));
        }

        if handler.load(Ordering::SeqCst).is_null() {
            return Err(MaskError::NoHandler(interrupt));
        }

        Ok(())
    }

    #[inline(always)]
    fn index_mie(&self, interrupt: Interrupt) -> (&plic::MIE, u32) {
        let nr = interrupt.into_bits() as usize;
        (&self.plic.mie[nr / 32], 1 << (nr % 32))
    }
}

/// Bit conversions
trait IntoBits: Sized + Copy {
    fn into_bits(self) -> u32;
}

trait TryFromBits: Sized + Copy {
    type Error;
    fn try_from_bits(bits: u32) -> Result<Self, Self::Error>;
}

impl IntoBits for Interrupt {
    fn into_bits(self) -> u32 {
        self as u8 as u32
    }
}

impl IntoBits for Priority {
    fn into_bits(self) -> u32 {
        u8::from(self) as u32
    }
}

impl TryFromBits for Priority {
    type Error = TryFromPriorityError;
    fn try_from_bits(bits: u32) -> Result<Self, Self::Error> {
        match bits {
            0 => Ok(Priority::P0),
            1 => Ok(Priority::P1),
            2 => Ok(Priority::P2),
            3 => Ok(Priority::P3),
            4 => Ok(Priority::P4),
            5 => Ok(Priority::P5),
            6 => Ok(Priority::P6),
            7 => Ok(Priority::P7),
            8 => Ok(Priority::P8),
            9 => Ok(Priority::P9),
            10 => Ok(Priority::P10),
            11 => Ok(Priority::P11),
            12 => Ok(Priority::P12),
            13 => Ok(Priority::P13),
            14 => Ok(Priority::P14),
            15 => Ok(Priority::P15),
            16 => Ok(Priority::P16),
            17 => Ok(Priority::P17),
            18 => Ok(Priority::P18),
            19 => Ok(Priority::P19),
            20 => Ok(Priority::P20),
            21 => Ok(Priority::P21),
            22 => Ok(Priority::P22),
            23 => Ok(Priority::P23),
            24 => Ok(Priority::P24),
            25 => Ok(Priority::P25),
            26 => Ok(Priority::P26),
            27 => Ok(Priority::P27),
            28 => Ok(Priority::P28),
            29 => Ok(Priority::P29),
            30 => Ok(Priority::P30),
            31 => Ok(Priority::P31),
            _ => Err(TryFromPriorityError(())),
        }
    }
}

struct Vectored {
    id: Option<Interrupt>,
    handler: AtomicPtr<()>,
}

impl Vectored {
    const fn new(interrupt: Interrupt) -> Self {
        Self {
            id: Some(interrupt),
            handler: AtomicPtr::new(null_mut()),
        }
    }

    const fn none() -> Self {
        Self {
            id: None,
            handler: AtomicPtr::new(no_such_interrupt as *mut fn() as *mut ()),
        }
    }
}

const INTERRUPT_LIST: &[Interrupt] = &[
    Interrupt::UART0,
    Interrupt::UART1,
    Interrupt::UART2,
    Interrupt::UART3,
    Interrupt::UART4,
    Interrupt::UART5,
    Interrupt::TWI0,
    Interrupt::TWI1,
    Interrupt::TWI2,
    Interrupt::TWI3,
    Interrupt::SPI0,
    Interrupt::SPI1,
    Interrupt::PWM,
    Interrupt::IR_TX,
    Interrupt::LEDC,
    Interrupt::OWA,
    Interrupt::DMIC,
    Interrupt::AUDIO_CODEC,
    Interrupt::I2S_PCM0,
    Interrupt::I2S_PCM1,
    Interrupt::I2S_PCM2,
    Interrupt::USB0_DEVICE,
    Interrupt::USB0_EHCI,
    Interrupt::USB0_OHCI,
    Interrupt::USB1_EHCI,
    Interrupt::USB1_OHCI,
    Interrupt::SMHC0,
    Interrupt::SMHC1,
    Interrupt::SMHC2,
    Interrupt::EMAC,
    Interrupt::DMAC_NS,
    Interrupt::CE_NS,
    Interrupt::SPINLOCK,
    Interrupt::HSTIMER0,
    Interrupt::HSTIMER1,
    Interrupt::GPADC,
    Interrupt::THS,
    Interrupt::TIMER0,
    Interrupt::TIMER1,
    Interrupt::LRADC,
    Interrupt::TPADC,
    Interrupt::WATCHDOG,
    Interrupt::IOMMU,
    Interrupt::GPIOB_NS,
    Interrupt::GPIOC_NS,
    Interrupt::GPIOD_NS,
    Interrupt::GPIOE_NS,
    Interrupt::GPIOF_NS,
    Interrupt::CSI_DMA0,
    Interrupt::CSI_DMA1,
    Interrupt::CSI_TOP_PKT,
    Interrupt::TVD,
    Interrupt::DSP_MBOX_RV_W,
    Interrupt::RV_MBOX_RV,
    Interrupt::RV_MBOX_DSP,
    Interrupt::IR_RX,
];

const N_INTERRUPTS: usize = {
    let mut max = 0;
    // INTERRUPT_LIST is sorted, but --- because we're doing this in a const fn
    // --- we may as well just find the actual max vector, instead of taking the
    // vector of the last element in the array; this will *always* be correct,
    // even if the interrupt list is ever out of order.
    //
    // Unfortunately, as you probably already know, const fn. So, we do this
    // with a goofy while loop, instead of `iter().max()` --- you can pretend it
    // says that, if you like.
    let mut i = 0;
    while i < INTERRUPT_LIST.len() {
        let vector = INTERRUPT_LIST[i] as usize;
        if vector > max {
            max = vector;
        }
        i += 1;
    }
    max + 1
};

static INTERRUPT_ARRAY: [Vectored; N_INTERRUPTS] = {
    // This is a static initializer, ignore the stupid goddamn clippy lint.
    #[allow(clippy::declare_interior_mutable_const)]
    const TRAP: Vectored = Vectored::none();
    let mut array = [TRAP; N_INTERRUPTS];

    // Populate the real interrupt vectors.
    let mut i = 0;
    while i < INTERRUPT_LIST.len() {
        let interrupt = INTERRUPT_LIST[i];
        let vector = interrupt as usize;
        array[vector] = Vectored::new(interrupt);
        i += 1;
    }
    array
};

fn no_such_interrupt() {
    panic!("no such interrupt!");
}
