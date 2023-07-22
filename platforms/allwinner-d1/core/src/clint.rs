use d1_pac::CLINT;

/// Core Local Interruptor (ClINT) interface
pub struct Clint {
    clint: CLINT,
}

impl Clint {
    /// Create a new `Clint` from the [`CLINT`](d1_pac::CLINT) peripheral
    pub fn new(clint: CLINT) -> Self {
        Self { clint }
    }

    /// Release the underlying [`CLINT`](d1_pac::CLINT) peripheral
    pub fn release(self) -> CLINT {
        self.clint
    }

    pub unsafe fn summon() -> Self {
        Self {
            clint: d1_pac::Peripherals::steal().CLINT,
        }
    }

    /// Get the (machine) time value.
    ///
    /// Note that the CLINT of the C906 core does not implement
    /// the `mtime` register and we need to get the time value
    /// with a CSR, which the `riscv` crate implements for us.
    pub fn get_mtime(&self) -> usize {
        riscv::register::time::read()
    }

    /// Set the machine time comparator.
    ///
    /// When `mtime` >= this value, a (machine) interrupt
    /// will be generated (if configured properly).
    pub fn set_mtimecmp(&self, cmp: usize) {
        let cmph = (cmp >> 32) as u32;
        let cmpl = (cmp & 0xffff_ffff) as u32;
        unsafe {
            self.clint.mtimecmph.write(|w| w.bits(cmph));
            self.clint.mtimecmpl.write(|w| w.bits(cmpl));
        }
    }

    /// Reset the machine time comparator to its default value.
    pub fn reset_mtimecmp(&self) {
        unsafe {
            self.clint.mtimecmph.write(|w| w.bits(0xffff_ffff));
            self.clint.mtimecmpl.write(|w| w.bits(0xffff_ffff));
        }
    }
}
