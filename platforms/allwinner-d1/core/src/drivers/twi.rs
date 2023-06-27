use d1_pac::{CCU, GPIO, TWI0};
use drivers::dmac::{
    descriptor::{
        AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType,
    },
    Channel, ChannelMode,
};
use kernel::{
    comms::{kchannel::KChannel, oneshot::Reusable},
    maitake::sync::WaitCell,
    mnemos_alloc::containers::FixedVec,
    registry::{uuid, Envelope, KernelHandle, Message, RegisteredDriver, ReplyTo, Uuid},
    Kernel,
};

pub struct Twi0 {
    _x: (),
}

impl Twi0 {
    /// Initialize TWI0 with the MangoPi MQ Pro pin mappings.
    pub unsafe fn mq_pro(twi: TWI0, ccu: &mut CCU, gpio: &mut GPIO) -> Self {
        // Initialization for TWI driver
        // Step 1: configure corresponding GPIO multiplex function as TWI mode
        gpio.pg_cfg1.modify(|_r, w| {
            // on the Mango Pi MQ Pro, the pi header's I2C0 pins are mapped to
            // TWI0 on PG12 and PG13:
            // https://mangopi.org/_media/mq-pro-sch-v12.pdf
            w.pg12_select().twi0_sck();
            w.pg13_select().twi0_sda();
            w
        });

        // TODO(eliza): do we need to disable pullups? The MQ Pro schematic
        // indicates that there's a 10k pullup on these pins...

        Self::init(twi, ccu)
    }

    /// Initialize TWI0 with the Lichee RV Dock pin mappings.
    pub unsafe fn lichee_rv(twi: TWI0, ccu: &mut CCU, gpio: &mut GPIO) -> Self {
        todo!("eliza: Lichee RV pin mappings")
    }

    /// This assumes the GPIO pin mappings are already configured.
    unsafe fn init(twi: TWI0, ccu: &mut CCU) -> Self {
        ccu.twi_bgr.modify(|_r, w| {
            // Step 2: Set TWI_BGR_REG[TWI(n)_GATING] to 0 to close TWI(n) clock.
            w.twi0_gating().mask();
            // Step 3: Set TWI_BGR_REG[TWI(n)_RST] to 0 to reset TWI(n) module.
            w.twi0_rst().assert();
            w
        });

        ccu.twi_bgr.modify(|_r, w| {
            // Step 3: Set TWI_BGR_REG[TWI(n)_RST] to 1 to reset TWI(n).
            w.twi0_rst().deassert();
            // Step 4: Set TWI_BGR_REG[TWI(n)_GATING] to 1 to open TWI(n) clock.
            w.twi0_gating().pass();
            w
        });

        twi.twi_drv_ctrl.modify(|_r, w| {
            // Step 5: Set TWI_DRV_CTRL[TWI_DRV_EN] to 1 to enable the TWI driver.
            w.twi_drv_en().enable();
            w
        });

        // Step 6: Set TWI_DRV_BUS_CTRL[CLK_M] and TWI_DRV_BUS_CTRL[CLK_N] to
        // get the needed rate (The clock source of TWI is from APB1).
        twi.twi_drv_bus_ctrl.modify(|_r, w| {
            // this makes it 400 kHz according to the datasheet; we could also
            // set CLK_M to 11 to get 100 kHz
            w.clk_m().bits(2);
            w.clk_n().bits(1);
            w
        });

        // Step 7: set TWI_DRV_CTRL[RESTART_MODE] to 0, and
        // TWI_DRV_CTRL[READ_TRAN_MODE] to 1, and set
        // TWI_DRV_INT_CTRL[TRAN_COM_INT_EN] to 1
        twi.twi_drv_ctrl.modify(|_r, w| {
            w.restart_mode().restart();
            w.read_tran_mode().not_send();
            w
        });
        twi.twi_drv_int_ctrl.modify(|_r, w| {
            w.tran_com_int_en().variant(true);
            w
        });

        // Step 8: When using DMA for data transmission, set
        // TWI_DRV_DMA_CFG[DMA_RX_EN] and TWI_DRV_DMA_CFG[DMA_TX_EN] to 1, and
        // configure TWI_DRV_DMA_CFG[RX_TRIG] and TWI_DRV_DMA_CFG[TX_TRIG] to
        // set the thresholds of RXFIFO and TXFIFO.
        twi.twi_drv_dma_cfg.modify(|_r, w| {
            w.dma_rx_en().bits(0x1);
            w.dma_tx_en().variant(true);
            // TODO(eliza): what are the thresholds for RXFIFO and TXFIFO?
            w
        });
        Twi0 { _x: () }
    }
}
