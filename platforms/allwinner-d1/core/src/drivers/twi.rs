//! Drivers for the Allwinner D1's I<sup>2</sup>C/TWI peripherals.
//!
//! I believe that the I<sup>2</sup>C controller used in the D1 is from the
//! Marvell MV64xxx family, although I'm not sure which one in particular. Linux
//! has a driver for this device, which can be found [here][linux-driver].
//!
//! [linux-driver]: https://github.com/torvalds/linux/blob/995b406c7e972fab181a4bb57f3b95e59b8e5bf3/drivers/i2c/busses/i2c-mv64xxx.c
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    task::{Poll, Waker},
};
use d1_pac::{twi, CCU, GPIO, TWI0};
use kernel::{
    comms::kchannel::{KChannel, KConsumer},
    embedded_hal_async::i2c::{ErrorKind, NoAcknowledgeSource},
    mnemos_alloc::containers::FixedVec,
    registry,
    services::i2c::{Addr, Direction, I2cService, Transaction, Transfer},
    trace::{self},
    Kernel,
};

/// TWI 0 configured in TWI engine mode.
pub struct Twi0Engine {
    twi: TWI0,
}

/// Data used by a TWI interrupt.
struct Twi {
    data: UnsafeCell<TwiData>,
}

struct TwiDataGuard<'a> {
    twi: &'a twi::RegisterBlock,
    data: &'a mut TwiData,
}

struct TwiData {
    state: State,
    op: TwiOp,
    err: Option<ErrorKind>,
    waker: Option<Waker>,
}

static TWI0_ISR: Twi = Twi {
    data: UnsafeCell::new(TwiData {
        state: State::Idle,
        op: TwiOp::None,
        err: None,
        waker: None,
    }),
};

enum TwiOp {
    Write {
        buf: FixedVec<u8>,
        pos: usize,
        len: usize,
        end: bool,
    },
    Read {
        buf: FixedVec<u8>,
        len: usize,
        read: usize,
        end: bool,
    },
    None,
}

/// TWI state machine
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum State {
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
    WaitForAck(Addr),
    /// Waiting for the target device to send a data byte.
    WaitForData(Addr),
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
        let _isr = kernel::isr::Isr::enter();
        // tracing::info!("TWI 0 interrupt");
        let twi = unsafe { &*TWI0::PTR };
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*TWI0_ISR.data.get())
        };

        data.advance_isr(twi);
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

        // soft reset bit
        twi.twi_srst.write(|w| w.soft_rst().set_bit());

        twi.twi_ccr.modify(|_r, w| {
            // according to the data sheet, setting CLK_M = 11, CLK_N = 1
            // means 100kHz.
            // setting CLK_M to 2 instead would get us 400kHz.
            w.clk_m().variant(2);
            w.clk_n().variant(1);
            w
        });

        // Step 6: Configure TWI_CNTR[BUS_EN] and TWI_CNTR[A_ACK], when using interrupt mode, set
        // TWI_CNTR[INT_EN] to 1, and register the system interrupt. In slave mode, configure TWI_ADDR and
        // TWI_XADDR registers to finish TWI initialization configuration
        twi.twi_cntr.write(|w| {
            w.bus_en().respond();
            w.a_ack().set_bit();
            w.m_stp().set_bit();
            w
        });

        // we only want to be the bus controller, so zero our address
        twi.twi_addr.write(|w| w.sla().variant(0));
        twi.twi_xaddr.write(|w| w.slax().variant(0));

        Self { twi }
    }

    pub async fn register(self, kernel: &'static Kernel, queued: usize) -> Result<(), ()> {
        let (tx, rx) = KChannel::new_async(queued).await.split();

        kernel.spawn(self.run(rx)).await;
        trace::debug!("TWI driver task spawned");
        kernel
            .with_registry(move |reg| reg.register_konly::<I2cService>(&tx).map_err(drop))
            .await?;

        Ok(())
    }

    #[tracing::instrument(name = "TWI", level = tracing::Level::INFO, skip(self, rx))]
    async fn run(self, rx: KConsumer<registry::Message<I2cService>>) {
        tracing::debug!("starting TWI driver task");
        while let Ok(registry::Message { msg, reply }) = rx.dequeue_async().await {
            let addr = msg.body.addr;
            let (txn, rx) = Transaction::new(msg.body).await;
            if let Err(error) = reply.reply_konly(msg.reply_with_body(|_| Ok(txn))).await {
                tracing::warn!(?error, "client hung up...");
            }
            self.transaction(addr, rx).await;
        }
    }

    #[tracing::instrument(level = tracing::Level::DEBUG, skip(self, txn))]
    async fn transaction(&self, addr: Addr, txn: KConsumer<Transfer>) {
        tracing::debug!("starting I2C transaction");
        let mut guard = TWI0_ISR.lock(&self.twi);

        // reset the TWI engine.
        guard.twi.twi_srst.write(|w| w.soft_rst().set_bit());
        guard.twi.twi_efr.reset();
        guard.twi.twi_data.reset();
        guard.twi.twi_cntr.modify(|_r, w| {
            w.bus_en().respond();
            w.m_stp().set_bit();
            w
        });

        let mut started = false;
        while let Ok(Transfer {
            buf,
            len,
            end,
            dir,
            rsp,
        }) = txn.dequeue_async().await
        {
            // setup TWI driver state for next op
            guard.data.state = if started {
                State::WaitForRestart(addr)
            } else {
                started = true;
                State::WaitForStart(addr)
            };
            guard.data.op = match dir {
                Direction::Read => {
                    // setup read op
                    tracing::debug!("reading {len} bytes");
                    TwiOp::Read {
                        buf,
                        len,
                        read: 0,
                        end,
                    }
                }
                Direction::Write => {
                    // setup write op
                    tracing::debug!("writing {len} bytes");
                    TwiOp::Write {
                        buf,
                        pos: 0,
                        len,
                        end,
                    }
                }
            };

            guard.wait_for_irq().await;
            tracing::debug!(?dir, "TWI operation completed");
            let res = if let Some(error) = guard.data.err.take() {
                tracing::warn!(?error, ?dir, "TWI error");
                Err(error)
            } else {
                match core::mem::replace(&mut guard.data.op, TwiOp::None) {
                    TwiOp::Read { buf, .. } => {
                        debug_assert_eq!(dir, Direction::Read);
                        Ok(buf)
                    }
                    TwiOp::Write { buf, .. } => {
                        debug_assert_eq!(dir, Direction::Write);
                        Ok(buf)
                    }
                    _ => unreachable!(),
                }
            };
            if rsp.send(res).is_err() {
                tracing::debug!("I2C transaction handle dropped");
                break;
            }
        }
        // transaction ended!
        tracing::debug!("I2C transaction ended");

        let guard = TWI0_ISR.lock(&self.twi);
        guard.twi.twi_cntr.modify(|_r, w| {
            w.m_stp().variant(true);
            w.a_ack().variant(false);
            w
        });
    }
}

impl Twi {
    #[must_use]
    fn lock<'a>(&'a self, twi: &'a twi::RegisterBlock) -> TwiDataGuard<'a> {
        // disable TWI interrupts while holding the guard.
        twi.twi_cntr.modify(|_r, w| w.int_en().low());
        let data = unsafe { &mut *(self.data.get()) };
        TwiDataGuard { data, twi }
    }
}

impl Drop for TwiDataGuard<'_> {
    fn drop(&mut self) {
        self.twi.twi_cntr.modify(|_r, w| w.int_en().high());
    }
}

impl TwiDataGuard<'_> {
    async fn wait_for_irq(&mut self) {
        let mut waiting = false;
        futures::future::poll_fn(|cx| {
            if waiting {
                self.twi.twi_cntr.modify(|_r, w| w.int_en().low());
                return Poll::Ready(());
            }

            self.data.waker = Some(cx.waker().clone());
            waiting = true;
            self.twi.twi_cntr.modify(|_r, w| {
                w.m_sta().set_bit();
                w.a_ack().set_bit();
                w.int_en().high();
                w
            });

            Poll::Pending
        })
        .await;
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

impl TwiData {
    fn advance_isr(&mut self, twi: &twi::RegisterBlock) {
        use status::*;

        for _ in 0..5 * 1000 {
            core::hint::spin_loop();
        }
        let status: u8 = twi.twi_stat.read().sta().bits();
        let mut needs_wake = false;
        tracing::info!(status = ?format_args!("{status:#x}"), state = ?self.state, "TWI interrupt");
        twi.twi_cntr.modify(|_cntr_r, cntr_w| {
            self.state = match self.state {
                State::Idle => {
                    // TODO: send a STOP?
                    cntr_w.m_stp().set_bit();
                    State::Idle
                }
                State::WaitForStart(addr) | State::WaitForRestart(addr)
                    if status == START_TRANSMITTED || status == REPEATED_START_TRANSMITTED =>
                {
                    let bits = {
                        // lowest bit is 1 if reading, 0 if writing.
                        let dir = match self.op {
                            TwiOp::Read { .. } => 0b1,
                            _ => 0b0,
                        };
                        match addr {
                            Addr::SevenBit(addr) => ((addr & 0x7f) << 1) | dir,
                            Addr::TenBit(_) => todo!("eliza: implement ten bit addrs"),
                        }
                    };
                    // send the address
                    twi.twi_data.write(|w| w.data().variant(bits));
                    State::WaitForAddr1Ack(addr)
                }
                // TODO(eliza): i think we can get rid of this code, make sure
                // this is the case!
                /*
                State::WaitForStart(addr)
                    // Sometimes when the TWI comes up, we will spuriously see this
                    // status, but it seems fine if we just ignore it once and then
                    // set the START bit again.
                    if status == status::ADDR1_WRITE_NACKED
                    // When connected to the Beepberry, we may sometimes come up
                    // in the ADDR1_WRITE_ACKED state. I don't know why this
                    // happens, possibly it's i2c-puppet's fault?
                    // TODO(eliza): figure that out...
                    || status == status::ADDR1_WRITE_ACKED
                => {
                    cntr_w.m_stp().set_bit();
                    cntr_w.m_sta().set_bit();
                    State::WaitForStart(addr)
                },
                // Same as above but we are waitig for a restart.
                State::WaitForRestart(addr)
                    if status == status::ADDR1_WRITE_NACKED
                    || status == status::ADDR1_WRITE_ACKED
                => {
                    cntr_w.m_sta().set_bit();
                    State::WaitForRestart(addr)
                }

                // Sometimes we get the interrupt with this bit set multiple times.
                State::WaitForAddr1Ack(addr) if status == REPEATED_START_TRANSMITTED => {
                    State::WaitForAddr1Ack(addr)
                }
                */
                State::WaitForAddr1Ack(Addr::SevenBit(addr)) if status == ADDR1_WRITE_ACKED =>
                // TODO(eliza): handle 10 bit addr...
                {
                    match &mut self.op {
                        TwiOp::Write { buf, ref mut pos, .. } => {
                            // send the first byte of data
                            let byte = buf.as_slice()[0];
                            tracing::debug!("TWI write data: {byte:#x}");
                            twi.twi_data.write(|w| w.data().variant(byte));
                            *pos += 1;
                            State::WaitForAck(Addr::SevenBit(addr))
                        },
                        TwiOp::Read { .. } => unreachable!(
                            "if we sent an address with a write bit, we should be in a write state (was Read)"
                        ),
                        TwiOp::None => unreachable!(
                            "if we sent an address with a write bit, we should be in a write state (was None)"
                        ),
                    }
                }
                State::WaitForAddr1Ack(Addr::SevenBit(addr)) if status == ADDR1_READ_ACKED =>
                // TODO(eliza): handle 10 bit addr...
                {
                    match self.op {
                        TwiOp::Read { .. } => State::WaitForData(Addr::SevenBit(addr)),
                        TwiOp::None => unreachable!(
                            "if we sent an address with a read bit, we should be in a read state (was None)"
                        ),
                        TwiOp::Write { .. } => unreachable!(
                            "if we sent an address with a read bit, we should be in a read state (was Write)"
                        ),
                    }
                }
                State::WaitForData(addr) if status == RX_DATA_ACKED || status == RX_DATA_NACKED => {
                    match &mut self.op {
                        &mut TwiOp::Read { ref mut buf, len, ref mut read, end } => {
                            let data = twi.twi_data.read().data().bits();
                            tracing::debug!("TWI read data: {data:#x}");
                            buf.try_push(data).expect("read buf should have space for data");
                            *read += 1;
                            let remaining = len - *read;
                            if remaining == 1 {
                                cntr_w.a_ack().clear_bit();
                            }
                            if remaining > 0 {
                                State::WaitForData(addr)
                            } else {
                                needs_wake = true;
                                // if this is the last operation in the
                                // transaction, send a STOP.
                                if end {
                                    cntr_w.m_stp().set_bit();
                                    State::Idle
                                } else {
                                    // otherwise, send a repeated START for the
                                    // next operation.
                                    cntr_w.m_sta().set_bit();
                                    State::WaitForRestart(addr)
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                // State::WaitForAck if status == ADDR1_WRITE_ACKED => State::WaitForAck,
                State::WaitForAck(addr) if status == TX_DATA_ACKED => {
                    match &mut self.op {
                        &mut TwiOp::Write { ref mut buf, ref mut pos, len, end } => {
                            if *pos == len {
                                needs_wake = true;
                                // Send a repeated START for the read portion of
                                // the transaction.
                                if end {
                                    cntr_w.m_stp().set_bit();
                                    State::Idle
                                } else {
                                    // otherwise, send a repeated START for the
                                    // next operation.
                                    cntr_w.m_sta().set_bit();
                                    tracing::debug!("TWI send restart");
                                    State::WaitForRestart(addr)
                                }
                            } else {
                                // Send the next byte of data
                                let byte = buf.as_slice()[*pos];
                                tracing::debug!("TWI write data: {byte:#x}");

                                twi.twi_data.write(|w| w.data().variant(byte));

                                *pos += 1;
                                State::WaitForAck(addr)
                            }
                        }
                        _ => unimplemented!(),
                    }
                }
                _ => {
                    let err = status::error(status);
                    kernel::trace::warn!(?err, status = ?format_args!("{status:#x}"), state = ?self.state, "TWI0 error");
                    self.err = Some(err);
                    cntr_w.m_stp().variant(true);
                    needs_wake = true;
                    State::Idle
                }
            };

            if needs_wake {
                if let Some(waker) = self.waker.take() {
                    waker.wake();
                    // If we are waking the driver task, we need to disable interrupts
                    // until the driver can prepare the next phase of the transaction.
                    cntr_w.int_en().low();
                }
            }

            // Writing back to the TWI_CNTR register *with the INT_FLAG bit
            // high* clears the interrupt. the D1 user manual never explains
            // this, but it's the same behavior as the DMAC interrupts, and the
            // Linux driver for the Marvell family mv64xxx has a special flag
            // which changes it to write back to TWI_CNTR with INT_FLAG set on
            // Allwinner hardware.
            cntr_w.int_flag().set_bit();
            cntr_w
        });
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
    pub(super) fn error(status: u8) -> ErrorKind {
        match status {
            ADDR1_WRITE_NACKED => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
            TX_DATA_NACKED => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data),
            ADDR1_READ_NACKED => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
            _ => ErrorKind::Other,
        }
    }

    /// 0x08: START condition transmitted
    pub const START_TRANSMITTED: u8 = 0x08;

    /// 0x10: Repeated START condition transmitted
    pub const REPEATED_START_TRANSMITTED: u8 = 0x10;
    /// 0x18: Address + Write bit transmitted, ACK received
    pub const ADDR1_WRITE_ACKED: u8 = 0x18;

    /// 0x20: Address + Write bit transmitted, ACK not received
    pub const ADDR1_WRITE_NACKED: u8 = 0x20;

    /// 0x28: Data byte transmitted in master mode, ACK received
    pub const TX_DATA_ACKED: u8 = 0x28;
    /// 0x30: Data byte transmitted in master mode, ACK not received
    pub const TX_DATA_NACKED: u8 = 0x30;

    pub const ADDR1_READ_ACKED: u8 = 0x40;
    pub const ADDR1_READ_NACKED: u8 = 0x48;

    /// 0x50: Data byte received in master mode, ACK transmitted
    pub const RX_DATA_ACKED: u8 = 0x50;

    /// 0x58: Data byte received in master mode, no ACK transmitted
    pub const RX_DATA_NACKED: u8 = 0x58;
}
