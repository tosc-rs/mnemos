//! This module provides a higher-level interface for the `Clock Controller Unit`.
use d1_pac::CCU;

pub struct Ccu {
    ccu: CCU,
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

// temporary
fn sdelay(delay_us: usize) {
    let clint = unsafe { crate::clint::Clint::summon() };
    let t = clint.get_mtime();
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

    /// This function is currently a copy from
    /// [xboot](https://github.com/xboot/xboot/blob/master/src/arch/riscv64/mach-d1/sys-clock.c)
    pub fn sys_clock_init(&mut self) {
        self.set_pll_cpux_axi();
        self.set_pll_periph0();
        self.set_ahb();
        self.set_apb();
        self.set_dma();
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

    fn set_dma(&mut self) {
        /* Dma reset */
        self.ccu.dma_bgr.modify(|_, w| w.rst().deassert());
        sdelay(20);
        /* Enable gating clock for dma */
        self.ccu.dma_bgr.modify(|_, w| w.gating().pass());
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
