//! Drivers for the Allwinner D1's I<sup>2</sup>C/TWI peripherals.
//!
//! I believe that the I<sup>2</sup>C controller used in the D1 is from the
//! Marvell MV64xxx family, although I'm not sure which one in particular. Linux
//! has a driver for this device, which can be found [here][linux-driver].
//!
//! [linux-driver]: https://github.com/torvalds/linux/blob/995b406c7e972fab181a4bb57f3b95e59b8e5bf3/drivers/i2c/busses/i2c-mv64xxx.c
use crate::dmac::{
    descriptor::{
        AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType,
    },
    Channel, ChannelMode,
};
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering},
};
use d1_pac::{twi, CCU, GPIO, TWI0};
use kernel::{
    buf::{ArrayBuf, OwnedReadBuf},
    embedded_hal_async::i2c::{ErrorKind, NoAcknowledgeSource},
    maitake::sync::{WaitCell, WaitQueue},
    mnemos_alloc::containers::FixedVec,
    services::i2c::Addr,
};
/// TWI 0 configured in TWI driver mode.
///
/// TWI driver mode is packet-oriented, and reads and writes to/from
/// target device registers *by register address*. This mode allows DMA
/// transfers to I2C devices.
// TODO(eliza): add TWI engine mode.
pub struct Twi0Driver {
    twi: TWI0,
    tx_chan: Channel,
    rx_chan: Channel,
}

/// TWI 0 configured in TWI engine mode.
pub struct Twi0Engine {
    twi: TWI0,
}

static TWI0_ENG_IRQ: WaitCell = WaitCell::new();

pub static TWI0_DRV_TX_DONE: WaitCell = WaitCell::new();
pub static TWI0_DRV_RX_DONE: WaitCell = WaitCell::new();

/// Data used by a TWI interrupt.
struct Twi {
    twi: &'static twi::RegisterBlock,
    data: UnsafeCell<TwiData>,
    waiter: WaitCell,
}

struct TwiDataGuard<'a> {
    twi: &'static twi::RegisterBlock,
    data: &'a mut TwiData,
}

struct TwiData {
    state: State,
    op: Op,
    err: Option<ErrorKind>,
}

static TWI0_ISR: Twi = Twi {
    twi: unsafe { &*TWI0::PTR },
    data: UnsafeCell::new(TwiData {
        state: State::Idle,
        op: Op::None,
        err: None,
    }),
    waiter: WaitCell::new(),
};

enum Op {
    Write {
        buf: FixedVec<u8>,
        pos: usize,
    },
    Read {
        buf: OwnedReadBuf,
        amt: usize,
        read: usize,
    },
    None,
}

/// TWI state machine
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum State {
    Invalid,
    Idle,
    /// Waiting for a `START` condition to be sent
    WaitForStart(Addr),
    /// Waiting for a restart.
    WaitForRestart(Addr),
    /// Waiting for the target device to `ACK` the first 7 bits of an addr.
    WaitForAddr1Ack(Addr),
    /// Waiting for the target device to `ACK` the second half of a 10-bit addr.
    WaitForAddr2Ack(Addr),
    /// Waiting for the target device to `ACK` a data byte.
    WaitForAck,
    /// Waiting for the target device to send a data byte.
    WaitForData,
}

impl Twi0Driver {
    /// Initialize TWI0 with the MangoPi MQ Pro pin mappings.
    pub unsafe fn mq_pro(
        twi: TWI0,
        ccu: &mut CCU,
        gpio: &mut GPIO,
        tx_chan: Channel,
        rx_chan: Channel,
    ) -> Self {
        // Initialization for TWI driver
        // Step 1: configure corresponding GPIO multiplex function as TWI mode
        pinmap_twi0_mq_pro(gpio);

        // TODO(eliza): do we need to disable pullups? The MQ Pro schematic
        // indicates that there's a 10k pullup on these pins...

        Self::init(twi, ccu, tx_chan, rx_chan)
    }

    /// Initialize TWI0 with the Lichee RV Dock pin mappings.
    pub unsafe fn lichee_rv(
        twi: TWI0,
        ccu: &mut CCU,
        gpio: &mut GPIO,
        tx_chan: Channel,
        rx_chan: Channel,
    ) -> Self {
        todo!("eliza: Lichee RV pin mappings")
    }

    /// This assumes the GPIO pin mappings are already configured.
    unsafe fn init(twi: TWI0, ccu: &mut CCU, tx_chan: Channel, rx_chan: Channel) -> Self {
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
            w.tran_err_int_en().variant(true);
            w
        });

        // Step 8: When using DMA for data transmission, set
        // TWI_DRV_DMA_CFG[DMA_RX_EN] and TWI_DRV_DMA_CFG[DMA_TX_EN] to 1, and
        // configure TWI_DRV_DMA_CFG[RX_TRIG] and TWI_DRV_DMA_CFG[TX_TRIG] to
        // set the thresholds of RXFIFO and TXFIFO.
        twi.twi_drv_dma_cfg.modify(|_r, w| {
            w.dma_rx_en().variant(1);
            w.dma_tx_en().variant(true);
            // TODO(eliza): what are the thresholds for RXFIFO and TXFIFO?
            w
        });
        Twi0Driver {
            twi,
            tx_chan,
            rx_chan,
        }
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
            config
                .try_into()
                .map_err(drop)
                .expect("bad descriptor config???")
        };
        unsafe {
            self.tx_chan
                .set_channel_modes(ChannelMode::Wait, ChannelMode::Wait);
            self.tx_chan.start_descriptor(NonNull::from(&descriptor));
        }

        self.twi.twi_drv_ctrl.modify(|_r, w| {
            w.start_tran().start();
            w
        });

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
        mut data: OwnedReadBuf,
        len: u16,
    ) -> OwnedReadBuf {
        self.twi.twi_drv_slv.write(|w| {
            // set target address
            match addr {
                Addr::SevenBit(addr) => {
                    w.slv_id().variant(addr);
                }
                Addr::TenBit(addr) => todo!("eliza: implement 10 bit addresses {addr:?}"),
            }

            // set command to 1 to indicate a read
            w.cmd().read();
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
            let destination = unsafe { data.unfilled_mut().as_mut_ptr().cast::<()>() };
            // XXX(eliza): is this correct???
            let source = self
                .twi
                .twi_drv_recv_fifo_acc
                .read()
                .recv_data_fifo()
                .bits() as *const ();
            let config = DescriptorConfig {
                source,
                destination,
                byte_counter: len as usize,
                link: None,
                wait_clock_cycles: 0,
                bmode: BModeSel::Normal,
                dest_width: DataWidth::Bit8,
                dest_addr_mode: AddressMode::LinearMode,
                dest_block_size: BlockSize::Byte1,
                dest_drq_type: DestDrqType::Dram,
                src_data_width: DataWidth::Bit8,
                src_addr_mode: AddressMode::LinearMode,
                src_block_size: BlockSize::Byte1,
                src_drq_type: SrcDrqType::Twi0,
            };
            config
                .try_into()
                .map_err(drop)
                .expect("bad descriptor config???")
        };
        unsafe {
            self.rx_chan
                .set_channel_modes(ChannelMode::Wait, ChannelMode::Wait);
            self.rx_chan.start_descriptor(NonNull::from(&descriptor));
        }

        self.twi.twi_drv_ctrl.modify(|_r, w| {
            w.start_tran().start();
            w
        });

        TWI0_DRV_RX_DONE
            .wait()
            .await
            .expect("TWI0_DRV_TX_DONE WaitCell should never be closed");
        unsafe {
            self.rx_chan.stop_dma();
        }

        data
    }
}

// === impl Twi0Engine ===

impl Twi0Engine {
    /// Initialize TWI0 in TWI engine mode, with the MangoPi MQ Pro pin mappings.
    pub unsafe fn mq_pro(twi: TWI0, ccu: &mut CCU, gpio: &mut GPIO) -> Self {
        pinmap_twi0_mq_pro(gpio);
        Self::init(twi, ccu)
    }

    /// Initialize TWI0 with the Lichee RV Dock pin mappings.
    pub unsafe fn lichee_rv(twi: TWI0, ccu: &mut CCU, gpio: &mut GPIO) -> Self {
        todo!("eliza: Lichee RV pin mappings")
    }

    /// Handle a TWI 0 interrupt
    pub fn handle_interrupt() {
        unsafe {
            TWI0_ISR.isr();
        }
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

        // Step 6: Configure TWI_CNTR[BUS_EN] and TWI_CNTR[A_ACK], when using interrupt mode, set
        // TWI_CNTR[INT_EN] to 1, and register the system interrupt. In slave mode, configure TWI_ADDR and
        // TWI_XADDR registers to finish TWI initialization configuration
        twi.twi_cntr.write(|w| {
            // enable bus responses.
            w.bus_en().respond();
            // enable auto-acknowledgement
            // w.a_ack().variant(true);
            w.m_stp().variant(true);
            // enable interrupts
            w.int_en().low();
            w
        });

        // we only want to be the bus controller, so zero our address
        twi.twi_addr.write(|w| w.sla().variant(0));
        twi.twi_xaddr.write(|w| w.slax().variant(0));

        Self { twi }
    }

    async fn wfi(&mut self) -> Result<u8, ErrorKind> {
        use core::{future::Future, task::Poll};
        let wait = TWI0_ENG_IRQ.wait();
        // register the waiter *before* we enable interrupts again
        futures::pin_mut!(wait);
        let mut needs_unmask = true;
        futures::future::poll_fn(|cx| match wait.as_mut().poll(cx) {
            Poll::Pending if needs_unmask => {
                needs_unmask = false;
                self.twi.twi_cntr.modify(|_r, w| {
                    // w.int_flag().clear_bit();
                    w.int_en().high();
                    w
                });
                Poll::Pending
            }
            x => return x,
        })
        .await;

        // wait.await.expect("cannot be closed");

        // let wait = TWI0_ENG_IRQ.wait().await;
        let stat = self.twi.twi_stat.read().bits();
        kernel::trace::info!("TWI0 stat: {:#x}", stat);
        let stat = match stat as u8 {
            // 0x00: Bus error
            bits if bits == 0x00 => Err(ErrorKind::Bus),
            // 0x08: START condition transmitted
            bits if bits == 0x08 => Ok(bits),
            // 0x10: Repeated START condition transmitted
            bits if bits == 0x10 => Ok(bits),
            // 0x18: Address + Write bit transmitted, ACK received
            bits if bits == 0x18 => Ok(bits),
            // 0x20: Address + Write bit transmitted, ACK not received
            bits if bits == 0x20 => Err(ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)),
            // 0x28: Data byte transmitted in master mode, ACK received
            bits if bits == 0x28 => Ok(bits),
            // 0x30: Data byte transmitted in master mode, ACK not received
            bits if bits == 0x30 => Err(ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data)),
            // 0x38: Arbitration lost in address or data byte
            bits if bits == 0x38 => Err(ErrorKind::ArbitrationLoss),
            // 0x40: Address + Read bit transmitted, ACK received
            bits if bits == 0x40 => Ok(bits),
            // 0x48: Address + Read bit transmitted, ACK not received
            bits if bits == 0x48 => Err(ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)),
            // 0x50: Data byte received in master mode, ACK transmitted
            bits if bits == 0x50 => Ok(bits),
            // 0x58: Data byte received in master mode, no ACK transmitted
            // XXX(eliza): is this an error? why would we not ack?
            bits if bits == 0x58 => Ok(bits),
            // 0x60: Slave address + Write bit received, ACK transmitted
            //
            // Note: this is technically not an error in theory, but this driver
            // only ever operates as the I2C controller, rather than the target.
            // So, if we see this status in the middle of a bus operation, we
            // were incorrectly operating in target mode?
            bits if bits == 0x60 => Err(ErrorKind::Other),
            // 0x68: Arbitration lost in the address as master, slave address +
            // Write bit received, ACK transmitted
            //
            // Note: again, this is not an error condition from the perspective
            // of the bus, but we expect to be the I2C controller.
            bits if bits == 0x68 => Err(ErrorKind::ArbitrationLoss),
            // 0x70: General Call address received, ACK transmitted
            // TODO(eliza): handle I2C general calls..
            bits if bits == 0x70 => Ok(bits),
            // 0x78: Arbitration lost in the address as master, General Call
            // address received, ACK transmitted
            // TODO(eliza): handle I2C general calls..
            bits if bits == 0x78 => Ok(bits),
            // 0x80: Data byte received after slave address received, ACK
            // transmitted
            bits if bits == 0x80 => Err(ErrorKind::Other),
            // 0x88: Data byte received after slave address received, not ACK
            // transmitted
            // 0x90: Data byte received after General Call received, ACK
            // transmitted
            // 0x80: Data byte received after slave address received, ACK
            // transmitted
            // 0x98: Data byte received after General Call received, not ACK
            // transmitted
            // 0xA0: STOP or repeated START condition received in slave mode
            // 0xA8: Slave address + Read bit received, ACK transmitted
            // 0xB0: Arbitration lost in the address as master, slave address +
            // Read bit received, ACK transmitted
            bits if bits == 0xb0 => Err(ErrorKind::ArbitrationLoss),
            // 0xB8: Data byte transmitted in slave mode, ACK received
            // 0xC0: Data byte transmitted in slave mode, ACK not received
            // 0xC8: The Last byte transmitted in slave mode, ACK received
            // 0xD0: Second Address byte + Write bit transmitted, ACK receive
            bits if bits == 0xd0 => Ok(bits),
            // 0xD8: Second Address byte + Write bit transmitted, ACK not
            // received
            bits if bits == 0xd8 => Err(ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)),
            // 0xF8: No relevant status information, INT_FLAG=0
            bits if bits == 0xf8 => Ok(bits),
            // any unrecognized status, or statuses related to us operating as
            // the I2C target, should return an error.
            _ => Err(ErrorKind::Other),
        };

        stat
    }

    async fn send_byte(&mut self, byte: u8) -> Result<(), ErrorKind> {
        self.twi.twi_data.write(|w| w.data().variant(byte));
        self.wfi().await?;
        Ok(())
    }

    async fn send_addr(&mut self, addr: Addr) -> Result<(), ErrorKind> {
        match addr {
            Addr::SevenBit(addr) => self.send_byte(addr).await?,
            Addr::TenBit(addr) => {
                let [low, high] = addr.to_le_bytes();
                self.send_byte(low).await?;
                self.send_byte(high).await?;
            }
        }
        Ok(())
    }

    pub async fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), ErrorKind> {
        // Step 1: Clear TWI_EFR register, and configure TWI_CNTR[M_STA] to 1 to
        // transmit the START signal.
        self.twi.twi_efr.reset();
        self.twi.twi_cntr.modify(|_r, w| {
            w.bus_en().respond();
            w.a_ack().variant(true);
            w.int_en().low();
            w
        });
        self.twi.twi_cntr.modify(|_r, w| w.m_sta().variant(true));

        // wait for an interrupt to confirm the transmission of the START
        // signal.
        // TODO(eliza): maybe check the status?
        let status = self.wfi().await;

        kernel::trace::info!("M_STA = 1");

        kernel::trace::info!("START interrupt");

        // Step 2: After the START signal is transmitted, the first interrupt is
        // triggered, then write device ID to TWI_DATA (For a 10-bit device ID,
        // firstly write the first byte ID, secondly write the second byte ID in
        // the next interrupt).
        self.send_addr(addr).await?;

        kernel::trace::info!("ADDR interrupted");

        // Step 3: Interrupt is triggered after data address transmission
        // completes, write data to be transmitted to TWI_DATA (For consecutive
        // write data operation, every byte transmission completion triggers
        // interrupt, during interrupt write the next byte data to TWI_DATA).
        for &byte in data {
            self.send_byte(byte).await?;
        }

        // Step 5: After transmission completes, write TWI_CNTR[M_STP] to 1 to
        // transmit the STOP signal and end this write-operation.
        self.twi.twi_cntr.modify(|_r, w| w.m_stp().variant(true));

        Ok(())
    }

    pub async fn read(
        &mut self,
        addr: Addr,
        buf: OwnedReadBuf,
        amt: usize,
    ) -> Result<(), ErrorKind> {
        {
            let guard = TWI0_ISR.lock();
            // Step 1: Clear TWI_EFR register, and set TWI_CNTR[A_ACK] to 1, and
            // configure TWI_CNTR[M_STA] to 1 to transmit the START signal.
            guard.twi.twi_efr.reset();
            guard.twi.twi_cntr.modify(|_r, w| {
                w.m_sta().variant(true);
                w.a_ack().variant(true);
                w
            });
            guard.data.state = State::WaitForAddr1Ack(addr);
            guard.data.op = Op::Read { buf, amt, read: 0 };
            // TODO(eliza): this is where we really need to be able to subscribe
            // to the WaitCell eagerly, *before* we drop the guard and unlock
            // the interrupt, so we don't race...
        };

        // // wait for an interrupt to confirm the transmission of the START
        // // signal.
        // // TODO(eliza): maybe check the status?
        // self.wfi().await?;

        // // Step 2: After the START signal is transmitted, the first interrupt is
        // // triggered, then write device ID to TWI_DATA (For a 10-bit device ID,
        // // firstly write the first byte ID, secondly write the second byte ID in
        // // the next interrupt).
        // self.send_addr(addr).await?;

        // // Step 4: The Interrupt is triggered after data address transmission
        // // completes, write TWI_CNTR[M_STA] to 1 to transmit new START signal,
        // // and after interrupt triggers, write device ID to TWI_DATA to start
        // // read-operation.
        // self.twi.twi_cntr.modify(|_r, w| w.m_sta().variant(true));
        // // XXX(eliza): is it really telling me to send the device addr twice???
        // self.send_addr(addr).await?;

        // // Step 5 After device address transmission completes, each receive
        // // completion will trigger an interrupt, in turn, read TWI_DATA to get
        // // data, when receiving the previous interrupt of the last byte data,
        // // clear TWI_CNTR[A_ACK] to stop acknowledge signal of the last byte.
        // for pos in dest {
        //     let byte = self.twi.twi_data.read().data().bits();
        //     pos.write(byte);
        //     self.wfi().await?;
        // }

        // self.twi.twi_cntr.modify(|_r, w| w.a_ack().variant(false));

        // // Step 6: Write TWI_CNTR[M_STP] to 1 to transmit the STOP signal and
        // // end this read-operation.
        // self.twi.twi_cntr.modify(|_r, w| w.m_stp().variant(true));

        Ok(())
    }
}

impl Twi {
    #[must_use]
    fn lock(&self) -> TwiDataGuard<'_> {
        // disable TWI interrupts while holding the guard.
        self.twi.twi_cntr.modify(|_r, w| w.int_en().low());
        let data = unsafe { &mut *(self.data.get()) };
        TwiDataGuard {
            data,
            twi: self.twi,
        }
    }
}

impl Drop for TwiDataGuard<'_> {
    fn drop(&mut self) {
        // now that we're done accessing the TWI data, we can re-enable the
        // interrupt.
        self.twi.twi_cntr.modify(|_r, w| w.int_en().high())
    }
}

impl Deref for TwiDataGuard<'_> {
    type Target = TwiData;

    fn deref(&self) -> &Self::Target {
        &*self.data
    }
}

impl DerefMut for TwiDataGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl Twi {
    unsafe fn isr(&self) {
        while self.twi.twi_cntr.read().int_flag() == true {
            self.advance_isr();
        }
    }

    unsafe fn advance_isr(&self) {
        use status::*;
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*self.data.get())
        };
        let status = self.twi.twi_stat.read().bits();
        tracing::info!(status = ?format_args!("{status:#x}"), state = ?data.state, "TWI interrupt");
        data.state = match data.state {
            State::Idle => {
                // TODO: send a STOP?
                State::Idle
            }
            State::WaitForStart(addr)
                if status == START_TRANSMITTED || status == REPEATED_START_TRANSMITTED =>
            {
                // send the address
                self.twi
                    .twi_data
                    .write(|w| w.data().variant(addr.low_bits()));
                State::WaitForAddr1Ack(addr)
            }
            State::WaitForAddr1Ack(Addr::SevenBit(_)) if status == ADDR1_ACKED =>
            // TODO(eliza): handle 10 bit addr...
            {
                match &mut data.op {
                    Op::None => unreachable!(),
                    Op::Write { buf, ref mut pos } => {
                        // send the first byte of data
                        self.twi
                            .twi_data
                            .write(|w| w.data().variant(buf.as_slice()[0]));
                        *pos += 1;
                        State::WaitForAck
                    }
                    Op::Read { .. } => State::WaitForData,
                }
            }
            State::WaitForData
                if status == RX_DATA_ACKED ||

            // XXX(eliza): is this an error? why would we not ack?
                status == RX_DATA_NACKED =>
            {
                match &mut data.op {
                    Op::Read { buf, amt, read } => {
                        let data = self.twi.twi_data.read().data().bits();
                        buf.copy_from_slice(&[data]);
                        *read += 1;
                        if read < amt {
                            State::WaitForData
                        } else {
                            self.waiter.wake();
                            // TODO(eliza): do we disable the IRQ until the
                            // waiter has advanced our state, in case it wants
                            // to read data...?
                            State::Idle
                        }
                    }
                    _ => unimplemented!(),
                }
            }
            State::WaitForAck if status == TX_DATA_ACKED => {
                match &mut data.op {
                    Op::Write { buf, pos } => {
                        if *pos < buf.as_slice().len() {
                            // send the next byte of data
                            self.twi
                                .twi_data
                                .write(|w| w.data().variant(buf.as_slice()[*pos]));
                            *pos += 1;
                            State::WaitForAck
                        } else {
                            self.waiter.wake();
                            // TODO(eliza): do we disable the IRQ until the
                            // waiter has advanced our state, in case it wants
                            // to read data...?
                            State::Idle
                        }
                    }
                    _ => unimplemented!(),
                }
            }
            _ => {
                data.err = Some(status::error(status));
                self.waiter.wake();
                State::Idle
            }
        };

        // self.twi.twi_cntr.modify(|_r, w| w.int_flag().variant(false));
    }
}

unsafe impl Sync for Twi {}

fn pinmap_twi0_mq_pro(gpio: &mut GPIO) {
    gpio.pg_cfg1.modify(|_r, w| {
        // on the Mango Pi MQ Pro, the pi header's I2C0 pins are mapped to
        // TWI0 on PG12 and PG13:
        // https://mangopi.org/_media/mq-pro-sch-v12.pdf
        w.pg12_select().twi0_sck();
        w.pg13_select().twi0_sda();
        w
    });
}

mod status {
    use super::*;
    pub(super) fn error(status: u32) -> ErrorKind {
        match status {
            ADDR1_NACKED => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
            TX_DATA_NACKED => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data),
            _ => ErrorKind::Other,
        }
    }

    /// 0x08: START condition transmitted
    pub const START_TRANSMITTED: u32 = 0x08;

    /// 0x10: Repeated START condition transmitted
    pub const REPEATED_START_TRANSMITTED: u32 = 0x10;

    /// 0x18: Address + Write bit transmitted, ACK received
    pub const ADDR1_ACKED: u32 = 0x18;

    /// 0x20: Address + Write bit transmitted, ACK not received
    pub const ADDR1_NACKED: u32 = 0x20;

    /// 0x28: Data byte transmitted in master mode, ACK received
    pub const TX_DATA_ACKED: u32 = 0x28;
    /// 0x30: Data byte transmitted in master mode, ACK not received
    pub const TX_DATA_NACKED: u32 = 0x30;

    /// 0x50: Data byte received in master mode, ACK transmitted
    pub const RX_DATA_ACKED: u32 = 0x50;

    /// 0x58: Data byte received in master mode, no ACK transmitted
    pub const RX_DATA_NACKED: u32 = 0x58;
}
