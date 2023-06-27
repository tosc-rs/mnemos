use d1_pac::{CCU, GPIO, TWI0};
use kernel::{
    drivers::i2c::Addr,
    buf::{OwnedReadBuf, ArrayBuf},
    maitake::sync::WaitCell,
};
use drivers::dmac::{
    descriptor::{
        AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType,
    },
    Channel, ChannelMode,
};
use core::ptr::NonNull;

/// TWI 0 configured in TWI driver mode.
///
/// TWI driver mode is packet-oriented, and reads and writes to/from
/// target device registers *by register address*. This mode allows DMA
/// transfers to I2C devices.
// TODO(eliza): add TWI engine mode.
pub struct Twi0Driver {
    twi: TWI0,
    tx_chan: Channel,
}

pub static TWI0_DRV_TX_DONE: WaitCell = WaitCell::new();

impl Twi0Driver {
    const TX_DMA_CHANNEL: u8 = 2;

    /// Initialize TWI0 with the MangoPi MQ Pro pin mappings.
    pub unsafe fn mq_pro(twi: TWI0, ccu: &mut CCU, gpio: &mut GPIO, tx_chan: Channel) -> Self {
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

        Self::init(twi, ccu, tx_chan)
    }

    /// Initialize TWI0 with the Lichee RV Dock pin mappings.
    pub unsafe fn lichee_rv(twi: TWI0, ccu: &mut CCU, gpio: &mut GPIO, tx_chan: Channel) -> Self {
        todo!("eliza: Lichee RV pin mappings")
    }

    /// This assumes the GPIO pin mappings are already configured.
    unsafe fn init(twi: TWI0, ccu: &mut CCU, tx_chan: Channel) -> Self {
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
        Twi0Driver { twi, tx_chan }
    }

    pub async fn write_register(
        &mut self,
        addr: Addr,
        register: u8,
        data: ArrayBuf<u8>,
        len: u16,
    ) -> ArrayBuf<u8> {
        self.twi.twi_drv_slv.write(|w| {
            // set target address
            match addr {
                Addr::SevenBit(addr) => {
                    w.slv_id().variant(addr);
                }
                Addr::TenBit(addr) => todo!("eliza: implement 10 bit addresses {addr:?}"),
            }

            // set command to 0 to indicate a write
            w.cmd().write();
            w
        });

        self.twi.twi_drv_fmt.write(|w| {
            // XXX(eliza): does this just disable the TWI driver's target
            // register address mode? i hope it does.
            // if this doesn't work we probably need to use a different
            // interface...
            w.addr_byte().variant(register);
            w.data_byte().variant(len);
            w
        });

        self.twi.twi_drv_cfg.modify(|_r, w| {
            w.packet_cnt().variant(1);
            w
        });

        // configure DMA channel
        let descriptor = {
            let (source_ptr, _) = data.ptrlen();
            let source = source_ptr.as_ptr().cast::<()>();
            // XXX(eliza): is this correct???
            let destination = self.twi.twi_drv_send_fifo_acc.as_ptr() as *mut ();
            let config = DescriptorConfig {
                source,
                destination,
                byte_counter: len as usize,
                link: None,
                wait_clock_cycles: 0,
                bmode: BModeSel::Normal,
                dest_width: DataWidth::Bit8,
                dest_addr_mode: AddressMode::IoMode,
                dest_block_size: BlockSize::Byte1,
                dest_drq_type: DestDrqType::Twi0,
                src_data_width: DataWidth::Bit8,
                src_addr_mode: AddressMode::LinearMode,
                src_block_size: BlockSize::Byte1,
                src_drq_type: SrcDrqType::Dram,
            };
            config.try_into().map_err(drop).expect("bad descriptor config???")
        };
        unsafe {
            self.tx_chan.set_channel_modes(ChannelMode::Wait, ChannelMode::Wait);
            self.tx_chan.start_descriptor(NonNull::from(&descriptor));
        }
        TWI0_DRV_TX_DONE
            .wait()
            .await
            .expect("TWI0_DRV_TX_DONE WaitCell should never be closed");
        unsafe {
            self.tx_chan.stop_dma();
        }
        data
    }

    pub async fn read_register(
        &mut self,
        addr: Addr,
        register: u8,
        data: OwnedReadBuf,
        len: u16,
    ) -> OwnedReadBuf {
        todo!("eliza")
    }
}
