use core::{sync::atomic::{AtomicPtr, Ordering}, ptr::null_mut};

use d1_pac::{plic, Interrupt, PLIC};

/// Interrupt Priority from 0..31
pub type Priority = plic::prio::PRIORITY_A;

#[doc = r" TryFromPrioritytError"]
#[derive(Debug, Copy, Clone)]
pub struct TryFromPriorityError(());

/// Platform-Level Interrupt Controller (PLIC) interface
pub struct Plic {
    plic: PLIC,
}

impl Plic {
    /// Create a new `Plic` from the [`PLIC`](d1_pac::PLIC) peripheral
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
        let nr = interrupt.into_bits() as usize;
        let (reg_offset, irq_en) = (nr / 32, 1 << (nr % 32));
        self.plic.mie[reg_offset].modify(|r, w| w.bits(r.bits() | irq_en));
    }

    /// Disable an interrupt
    pub fn mask(&self, interrupt: Interrupt) {
        let nr = interrupt.into_bits() as usize;
        let (reg_offset, irq_en) = (nr / 32, 1 << (nr % 32));
        self.plic.mie[reg_offset].modify(|r, w| unsafe { w.bits(r.bits() & !irq_en) });
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
            Err(_) => {
                panic!("error claiming interrupt");
            }
        }
    }

    pub fn complete(&self, interrupt: Interrupt) {
        self.plic
            .mclaim
            .write(|w| w.mclaim().variant(interrupt.into_bits() as u16));
    }

    pub unsafe fn register(&self, interrupt: Interrupt, new_hdl: fn()) {
        let v = INTERRUPT_ARRAY.iter().find(|v| v.id == interrupt as u16);
        if let Some(Vectored { id: _id, handler }) = v {
            handler.store(new_hdl as *mut fn() as *mut (), Ordering::Release);
        }
    }

    pub unsafe fn activate(&self, interrupt: Interrupt, prio: Priority) -> Result<(), ()> {
        let v = INTERRUPT_ARRAY.iter().find(|v| v.id == interrupt as u16);
        if let Some(v) = v {
            if !v.handler.load(Ordering::SeqCst).is_null() {
                self.set_priority(interrupt, prio);
                self.unmask(interrupt);
                return Ok(());
            }
        }
        Err(())
    }

    pub fn deactivate(&self, interrupt: Interrupt) -> Result<(), ()> {
        let v = INTERRUPT_ARRAY.iter().find(|v| v.id == interrupt as u16);
        if let Some(v) = v {
            if !v.handler.load(Ordering::SeqCst).is_null() {
                self.mask(interrupt);
                return Ok(());
            }
        }
        Err(())
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
    id: u16,
    handler: AtomicPtr<()>,
}

impl Vectored {
    const fn new(id: u16) -> Self {
        Self {
            id,
            handler: AtomicPtr::new(null_mut()),
        }
    }

    const fn from_interrupt(i: Interrupt) -> Self {
        Self::new(i as u16)
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

const fn lister() -> [Vectored; INTERRUPT_LIST.len()] {
    const ONE: Vectored = Vectored::new(0);
    let mut arr = [ONE; INTERRUPT_LIST.len()];
    let mut i = 0;
    while i < INTERRUPT_LIST.len() {
        // Just take the ID,
        arr[i] = Vectored::from_interrupt(INTERRUPT_LIST[i]);
        i += 1;
    }
    arr
}

static INTERRUPT_ARRAY: [Vectored; INTERRUPT_LIST.len()] = lister();

// TODO: This is re-inventing vector tables, which I think we might be able to
// do at a hardware level. For now, it's probably fine
#[export_name = "MachineExternal"]
fn im_an_interrupt() {
    let plic = unsafe { Plic::summon() };
    let claim = plic.claim();
    let claim_u16 = claim as u16;

    // Is this a known interrupt?
    let handler = INTERRUPT_ARRAY.iter().find(|i| i.id == claim_u16);
    if let Some(Vectored { id: _id, handler }) = handler {
        let ptr = handler.load(Ordering::SeqCst); // todo: ordering
        if !ptr.is_null() {
            let hdlr: fn() = unsafe { core::mem::transmute(ptr) };
            (hdlr)();
        } // TODO: panic on else?
    } // TODO: panic on else?


    // Release claim
    plic.complete(claim);
}
