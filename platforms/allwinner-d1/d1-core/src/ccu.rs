//! This module provides a higher-level interface for the Clock Controller Unit (CCU).
//!
//! ## Provenance
//!
//! Portions of this file were ported from the `xboot` project. These portions were used
//! under the terms of the MIT license. Refer to `LICENSE-MIT` at top of this repo for
//! full license text. `xboot` copyright information:
//!
//! Copyright(c) 2007-2022 Jianjun Jiang <8192542@qq.com>
use d1_pac::CCU;
use d1_pac::DMAC;
use d1_pac::{SMHC0, SMHC1, SMHC2};
use d1_pac::{SPI0, SPI_DBI};
use d1_pac::{TWI0, TWI1, TWI2, TWI3};
use d1_pac::{UART0, UART1, UART2, UART3, UART4, UART5};

pub struct Ccu {
    ccu: CCU,
}

#[derive(PartialEq)]
pub enum BusGating {
    Mask,
    Pass,
}

#[derive(PartialEq)]
pub enum BusReset {
    Assert,
    Deassert,
}

/// Trait to be implemented for module clocks that can be gated and reset
pub trait BusGatingResetRegister {
    /// Enable or disable the clock gating bit
    fn gating(ccu: &mut CCU, gating: BusGating);
    /// Enable or disable the clock reset bit
    fn reset(ccu: &mut CCU, reset: BusReset);
}

// TODO: should this move into the `Clint`?
fn sdelay(delay_us: usize) {
    let clint = unsafe { crate::clint::Clint::summon() };
    let t = clint.get_mtime();
    // TODO: verify that mtime sourced directly from DXCO (24 MHz)
    while clint.get_mtime() < (t + 24 * delay_us) {
        core::hint::spin_loop()
    }
}

impl Ccu {
    #[must_use]
    pub fn new(ccu: CCU) -> Self {
        Self { ccu }
    }

    #[must_use]
    pub fn release(self) -> CCU {
        self.ccu
    }

    /// De-assert the reset bit and enable the clock gating bit for the given module
    pub fn enable_module<MODULE: BusGatingResetRegister>(&mut self, _mod: &mut MODULE) {
        MODULE::reset(&mut self.ccu, BusReset::Deassert);
        sdelay(20);
        MODULE::gating(&mut self.ccu, BusGating::Pass);
    }

    /// Disable the clock gating bit and assert the reset bit for the given module
    pub fn disable_module<MODULE: BusGatingResetRegister>(&mut self, _mod: &mut MODULE) {
        MODULE::gating(&mut self.ccu, BusGating::Mask);
        // TODO: delay?
        MODULE::reset(&mut self.ccu, BusReset::Assert);
    }

    /// Allow modules to configure their own clock on a PAC level
    // TODO: find a good abstraction so we don't need this anymore
    pub fn borrow_raw(&mut self) -> &mut CCU {
        &mut self.ccu
    }

    /// Initialize the system clocks to the same default value that is also set by `xfel`
    pub fn sys_clock_init(&mut self) {
        // The clock initialization functions are ported to Rust, based on the C implementation in
        // [xboot](https://github.com/xboot/xboot/blob/master/src/arch/riscv64/mach-d1/sys-clock.c)
        self.set_pll_cpux_axi();
        self.set_pll_periph0();
        self.set_ahb();
        self.set_apb();
        // This is where `xboot` resets and enables the DMA bus gating register.
        // We don't do that, because this is performed by the DMAC driver's
        // initialization function, instead.
        self.set_mbus();

        macro_rules! set_module {
            ($self: ident, $module: ident) => {
                if ($self.ccu.$module.read().pll_en().bit_is_clear()) {
                    $self.ccu.$module.modify(|_, w| {
                        w.pll_ldo_en().enable();
                        w.pll_en().enable();
                        w
                    });

                    $self.ccu.$module.modify(|_, w| w.lock_enable().enable());

                    while $self.ccu.$module.read().lock().bit_is_clear() {
                        core::hint::spin_loop();
                    }
                    sdelay(20);

                    $self.ccu.$module.modify(|_, w| w.lock_enable().disable());
                }
            };
        }

        set_module!(self, pll_peri_ctrl);
        set_module!(self, pll_video0_ctrl);
        set_module!(self, pll_video1_ctrl);
        set_module!(self, pll_ve_ctrl);
        set_module!(self, pll_audio0_ctrl);
        set_module!(self, pll_audio1_ctrl);
    }

    fn set_pll_cpux_axi(&mut self) {
        // Select DCXO (24 MHz) as CPU clock source.
        // AXI divide ratio is 3, system APB clock ratio is 4.
        self.ccu.riscv_clk.write(|w| {
            w.clk_src_sel().hosc();
            w.axi_div_cfg().variant(3);
            w.div_cfg().variant(1);
            w
        });
        sdelay(1);

        // Disable PLL gating
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.pll_output_gate().disable());

        // Enable PLL LDO
        self.ccu.pll_cpu_ctrl.modify(|_, w| w.pll_ldo_en().enable());
        sdelay(5);

        // Set default clock to 1008 MHz
        self.ccu.pll_cpu_ctrl.modify(|r, w| {
            // undocumented part of register is cleared by xboot
            unsafe { w.bits(r.bits() & !(0x3 << 16)) };
            w.pll_m().variant(0);
            w.pll_n().variant(41);
            w
        });

        // Lock the CPU PLL
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.lock_enable().enable());

        // Enable PLL
        self.ccu.pll_cpu_ctrl.modify(|_, w| w.pll_en().enable());

        // Wait until PLL is stable
        while self.ccu.pll_cpu_ctrl.read().lock().bit_is_clear() {
            core::hint::spin_loop();
        }
        sdelay(20);

        // Enable PLL gating
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.pll_output_gate().enable());

        // Unlock the CPU PLL
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.lock_enable().disable());
        sdelay(1);

        // Change the CPU clock source to PLL_CPU.
        // Sets the RISC-V clock to 1008 MHz and the RISC-V AXI clock to 504 MHz.
        self.ccu.riscv_clk.modify(|_, w| {
            w.clk_src_sel().pll_cpu();
            w.axi_div_cfg().variant(1);
            w.div_cfg().variant(0);
            w
        });
        sdelay(1);
    }

    fn set_pll_periph0(&mut self) {
        // Check if PLL_PERI has already been enabled
        if self.ccu.pll_peri_ctrl.read().pll_en().bit_is_set() {
            return;
        }

        // Change PSI source to DCXO (24 MHz)
        self.ccu.psi_clk.modify(|_, w| w.clk_src_sel().hosc());

        // Set the default value for PLL_N
        self.ccu.pll_peri_ctrl.write(|w| w.pll_n().variant(0x63));

        // Lock the PLL
        self.ccu
            .pll_peri_ctrl
            .modify(|_, w| w.lock_enable().enable());

        // Enable 'PLL_PERI(1X)', 'PLL_PERI(2X)' and 'PLL_PERI(800M)'
        self.ccu.pll_peri_ctrl.modify(|_, w| w.pll_en().enable());

        // Wait until PLL is stable
        while self.ccu.pll_peri_ctrl.read().lock().bit_is_clear() {
            core::hint::spin_loop();
        }
        sdelay(20);

        // Unlock the PLL
        self.ccu
            .pll_peri_ctrl
            .modify(|_, w| w.lock_enable().disable());
    }

    fn set_ahb(&mut self) {
        // This could potentially be done in a single write, but we follow
        // the `xboot` implementation which also splits this in 2 operations.
        self.ccu
            .psi_clk
            .write(|w| w.factor_m().variant(2).factor_n().n1());
        self.ccu
            .psi_clk
            .modify(|_, w| w.clk_src_sel().pll_peri_1x());
        sdelay(1);
    }

    fn set_apb(&mut self) {
        // This could potentially be done in a single write, but we follow
        // the `xboot` implementation which also splits this in 2 operations.
        self.ccu.apb_clk[0].write(|w| w.factor_m().variant(2).factor_n().n2());
        self.ccu.apb_clk[0].modify(|_, w| w.clk_src_sel().pll_peri_1x());
        sdelay(1);
    }

    fn set_mbus(&mut self) {
        // Reset the MBUS domain
        self.ccu.mbus_clk.modify(|_, w| w.mbus_rst().deassert());
        sdelay(1);
        // Enable MBUS master clock gating
        self.ccu.mbus_mat_clk_gating.write(|w| {
            w.dma_mclk_en().pass();
            w.ve_mclk_en().pass();
            w.ce_mclk_en().pass();
            w.tvin_mclk_en().pass();
            w.csi_mclk_en().pass();
            w.g2d_mclk_en().pass();
            w.riscv_mclk_en().pass();
            w
        });
    }
}

macro_rules! impl_bgr {
    ($($MODULE:ident : ($reg:ident, $gating:ident, $reset:ident),)+) => {
        $(
            impl BusGatingResetRegister for $MODULE {
                fn gating(ccu: &mut CCU, gating: BusGating) {
                    ccu.$reg.modify(|_, w| {
                        w.$gating().bit(gating == BusGating::Pass)
                    });
                }

                fn reset(ccu: &mut CCU, reset: BusReset) {
                    ccu.$reg.modify(|_, w| {
                        w.$reset().bit(reset == BusReset::Deassert)
                    });
                }
            }
        )+
    }
}

impl_bgr! {
    DMAC:    (dma_bgr, gating, rst),
    SMHC0:   (smhc_bgr, smhc0_gating, smhc0_rst),
    SMHC1:   (smhc_bgr, smhc1_gating, smhc1_rst),
    SMHC2:   (smhc_bgr, smhc2_gating, smhc2_rst),
    SPI0:    (spi_bgr, spi0_gating, spi0_rst),
    SPI_DBI: (spi_bgr, spi1_gating, spi1_rst),
    TWI0:    (twi_bgr, twi0_gating, twi0_rst),
    TWI1:    (twi_bgr, twi1_gating, twi1_rst),
    TWI2:    (twi_bgr, twi2_gating, twi2_rst),
    TWI3:    (twi_bgr, twi3_gating, twi3_rst),
    UART0:   (uart_bgr, uart0_gating, uart0_rst),
    UART1:   (uart_bgr, uart1_gating, uart1_rst),
    UART2:   (uart_bgr, uart2_gating, uart2_rst),
    UART3:   (uart_bgr, uart3_gating, uart3_rst),
    UART4:   (uart_bgr, uart4_gating, uart4_rst),
    UART5:   (uart_bgr, uart5_gating, uart5_rst),
}
