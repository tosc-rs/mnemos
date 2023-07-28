//! This module provides a higher-level interface for the `Clock Controller Unit`.
use d1_pac::CCU;
use d1_pac::DMAC;
use d1_pac::{SMHC0, SMHC1, SMHC2};
use d1_pac::{SPI0, SPI_DBI};
use d1_pac::{TWI0, TWI1, TWI2, TWI3};
use d1_pac::{UART0, UART1, UART2, UART3, UART4, UART5};

pub struct Ccu {
    ccu: CCU,
}

/// Trait to be implemented for module clocks that can be gated and reset
pub trait BusGatingResetRegister {
    /// Enable or disable the clock reset bit
    fn gating(ccu: &mut CCU, pass: bool);
    /// Enable or disable the clock gating bit
    fn reset(ccu: &mut CCU, deassert: bool);
}

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

// TODO: should this move into the `Clint`?
fn sdelay(delay_us: usize) {
    let clint = unsafe { crate::clint::Clint::summon() };
    let t = clint.get_mtime();
    // TODO: confirm clock source of mtime
    while clint.get_mtime() < (t + 24 * delay_us) {
        core::hint::spin_loop()
    }
}

impl Ccu {
    pub fn new(ccu: CCU) -> Self {
        Self { ccu }
    }

    pub fn release(self) -> CCU {
        self.ccu
    }

    /// De-assert the reset bit and enable the clock gating bit for the given module
    pub fn enable_module<MODULE: BusGatingResetRegister>(&mut self) {
        MODULE::reset(&mut self.ccu, true);
        sdelay(20);
        MODULE::gating(&mut self.ccu, true);
    }

    /// Disable the clock gating bit and assert the reset bit for the given module
    pub fn disable_module<MODULE: BusGatingResetRegister>(&mut self) {
        MODULE::gating(&mut self.ccu, false);
        // TODO: delay?
        MODULE::reset(&mut self.ccu, false);
    }

    /// Allow modules to configure their own clock on a PAC level
    // TODO: find a good abstraction so we don't need this anymore
    pub fn borrow(&mut self) -> &mut CCU {
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
        self.set_mbus();
        set_module!(self, pll_peri_ctrl);
        set_module!(self, pll_video0_ctrl);
        set_module!(self, pll_video1_ctrl);
        set_module!(self, pll_ve_ctrl);
        set_module!(self, pll_audio0_ctrl);
        set_module!(self, pll_audio1_ctrl);
    }

    fn set_pll_cpux_axi(&mut self) {
        /* Select cpux clock src to osc24m, axi divide ratio is 3, system apb clk ratio is 4 */
        self.ccu.riscv_clk.write(|w| {
            w.clk_src_sel().hosc();
            w.axi_div_cfg().variant(3);
            w.div_cfg().variant(1);
            w
        });
        sdelay(1);

        /* Disable pll gating */
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.pll_output_gate().disable());

        /* Enable pll ldo */
        self.ccu.pll_cpu_ctrl.modify(|_, w| w.pll_ldo_en().enable());
        sdelay(5);

        /* Set default clk to 1008mhz */
        self.ccu.pll_cpu_ctrl.modify(|r, w| {
            // undocumented part of register is cleared by xboot
            unsafe { w.bits(r.bits() & !(0x3 << 16)) };
            w.pll_m().variant(0);
            w.pll_n().variant(41);
            w
        });

        /* Lock enable */
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.lock_enable().enable());

        /* Enable pll */
        self.ccu.pll_cpu_ctrl.modify(|_, w| w.pll_en().enable());

        /* Wait pll stable */
        while self.ccu.pll_cpu_ctrl.read().lock().bit_is_clear() {
            core::hint::spin_loop();
        }
        sdelay(20);

        /* Enable pll gating */
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.pll_output_gate().enable());

        /* Lock disable */
        self.ccu
            .pll_cpu_ctrl
            .modify(|_, w| w.lock_enable().disable());
        sdelay(1);

        /* Set and change cpu clk src */
        self.ccu.riscv_clk.modify(|_, w| {
            w.clk_src_sel().pll_cpu();
            w.axi_div_cfg().variant(1);
            w.div_cfg().variant(0);
            w
        });
        sdelay(1);
    }

    fn set_pll_periph0(&mut self) {
        /* Periph0 has been enabled */
        if self.ccu.pll_peri_ctrl.read().pll_en().bit_is_set() {
            return;
        }

        /* Change psi src to osc24m */
        self.ccu.psi_clk.modify(|_, w| w.clk_src_sel().hosc());

        /* Set default val */
        self.ccu.pll_peri_ctrl.write(|w| w.pll_n().variant(0x63));

        /* Lock enable */
        self.ccu
            .pll_peri_ctrl
            .modify(|_, w| w.lock_enable().enable());

        /* Enabe pll 600m(1x) 1200m(2x) */
        self.ccu.pll_peri_ctrl.modify(|_, w| w.pll_en().enable());

        /* Wait pll stable */
        while self.ccu.pll_peri_ctrl.read().lock().bit_is_clear() {
            core::hint::spin_loop();
        }
        sdelay(20);

        /* Lock disable */
        self.ccu
            .pll_peri_ctrl
            .modify(|_, w| w.lock_enable().disable());
    }

    fn set_ahb(&mut self) {
        self.ccu
            .psi_clk
            .write(|w| w.factor_m().variant(2).factor_n().n1());
        self.ccu
            .psi_clk
            .modify(|_, w| w.clk_src_sel().pll_peri_1x());
        sdelay(1);
    }

    fn set_apb(&mut self) {
        self.ccu.apb_clk[0].write(|w| w.factor_m().variant(2).factor_n().n2());
        self.ccu.apb_clk[0].modify(|_, w| w.clk_src_sel().pll_peri_1x());
        sdelay(1);
    }

    fn set_mbus(&mut self) {
        /* Reset mbus domain */
        self.ccu.mbus_clk.modify(|_, w| w.mbus_rst().deassert());
        sdelay(1);
        /* Enable mbus master clock gating */
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
                fn gating(ccu: &mut CCU, pass: bool) {
                    ccu.$reg.modify(|_, w| w.$gating().bit(pass));
                }

                fn reset(ccu: &mut CCU, deassert: bool) {
                    ccu.$reg.modify(|_, w| w.$reset().bit(deassert));
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
