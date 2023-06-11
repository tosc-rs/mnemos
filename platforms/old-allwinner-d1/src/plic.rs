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
