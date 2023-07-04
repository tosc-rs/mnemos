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
};
use d1_pac::{twi, CCU, GPIO, TWI0};
use kernel::{
    buf::OwnedReadBuf,
    embedded_hal_async::i2c::{ErrorKind, NoAcknowledgeSource},
    maitake::sync::WaitCell,
    mnemos_alloc::containers::FixedVec,
    services::i2c::Addr,
};

/// TWI 0 configured in TWI engine mode.
pub struct Twi0Engine {
    twi: TWI0,
}

/// Data used by a TWI interrupt.
struct Twi {
    data: UnsafeCell<TwiData>,
    waiter: WaitCell,
}

struct TwiDataGuard<'a> {
    twi: &'a twi::RegisterBlock,
    data: &'a mut TwiData,
}

struct TwiData {
    state: State,
    op: Op,
    err: Option<ErrorKind>,
}

static TWI0_ISR: Twi = Twi {
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
        // tracing::info!("TWI 0 interrupt");
        let twi = unsafe { &*TWI0::PTR };
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*TWI0_ISR.data.get())
        };

        data.advance_isr(twi, &TWI0_ISR.waiter);
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

        twi.twi_ccr.modify(|_r, w| {
            // according to the data sheet, setting CLK_M = 11, CLK_N = 1
            // means 100kHz.
            // setting CLK_M to 2 instead would get us 400kHz.
            w.clk_m().variant(11);
            w.clk_n().variant(1);
            w
        });

        // Step 6: Configure TWI_CNTR[BUS_EN] and TWI_CNTR[A_ACK], when using interrupt mode, set
        // TWI_CNTR[INT_EN] to 1, and register the system interrupt. In slave mode, configure TWI_ADDR and
        // TWI_XADDR registers to finish TWI initialization configuration
        twi.twi_cntr.write(|w| {
            // enable bus responses.
            w.bus_en().respond();
            // enable auto-acknowledgement
            w.a_ack().variant(true);
            w.m_stp().variant(true);
            // enable interrupts
            w.int_en().high();
            w
        });

        // hopefully this is basically the same as udelay(5)
        for _ in 0..5 * 1000 {
            core::hint::spin_loop();
        }

        // // we only want to be the bus controller, so zero our address
        // twi.twi_addr.write(|w| w.sla().variant(0));
        // twi.twi_xaddr.write(|w| w.slax().variant(0));

        Self { twi }
    }

    pub async fn write(
        &mut self,
        addr: Addr,
        buf: FixedVec<u8>,
    ) -> Result<FixedVec<u8>, ErrorKind> {
        // tracing::info!("start twi write");
        let guard = TWI0_ISR.lock(&self.twi);
        // Step 1: Clear TWI_EFR register, and set TWI_CNTR[A_ACK] to 1, and
        // configure TWI_CNTR[M_STA] to 1 to transmit the START signal.
        guard.twi.twi_efr.reset();
        guard.twi.twi_cntr.modify(|_r, w| w.a_ack().variant(true));
        guard
            .twi
            .twi_cntr
            .modify(|_r, w: &mut twi::twi_cntr::W| w.m_sta().variant(true));
        guard.data.state = State::WaitForStart(addr);
        guard.data.op = Op::Write { buf, pos: 0 };
        // TODO(eliza): this is where we really need to be able to subscribe
        // to the WaitCell eagerly, *before* we drop the guard and unlock
        // the interrupt, so we don't race...

        let wait = TWI0_ISR.waiter.wait();
        futures::pin_mut!(wait);
        futures::poll!(&mut wait);
        drop(guard);
        wait.await;
        kernel::trace::info!("wait returned");

        let guard = TWI0_ISR.lock(&self.twi);

        // guard.twi.twi_cntr.modify(|_r, w| {
        //     w.m_stp().variant(true);
        //     w.a_ack().variant(false);
        //     w
        // });
        let res = if let Some(err) = guard.data.err.take() {
            Err(err)
        } else {
            match core::mem::replace(&mut guard.data.op, Op::None) {
                Op::Write { buf, .. } => Ok(buf),
                _ => unreachable!(),
            }
        };
        core::mem::forget(guard);
        res
    }

    pub async fn read(
        &mut self,
        addr: Addr,
        buf: OwnedReadBuf,
        amt: usize,
    ) -> Result<OwnedReadBuf, ErrorKind> {
        let guard = TWI0_ISR.lock(&self.twi);
        // Step 1: Clear TWI_EFR register, and set TWI_CNTR[A_ACK] to 1, and
        // configure TWI_CNTR[M_STA] to 1 to transmit the START signal.
        guard.twi.twi_efr.reset();
        guard.twi.twi_cntr.modify(|_r, w| {
            w.m_sta().variant(true);
            w.a_ack().variant(true);
            w
        });
        guard.data.state = State::WaitForStart(addr);
        guard.data.op = Op::Read { buf, amt, read: 0 };
        // TODO(eliza): this is where we really need to be able to subscribe
        // to the WaitCell eagerly, *before* we drop the guard and unlock
        // the interrupt, so we don't race...

        let wait = TWI0_ISR.waiter.wait();
        futures::pin_mut!(wait);
        let poll = futures::poll!(&mut wait);
        kernel::trace::info!(?poll);
        drop(guard);
        // TWI0_ISR.waiter.wait().await;
        wait.await.unwrap();
        kernel::trace::info!("read wait returned");

        let guard = TWI0_ISR.lock(&self.twi);

        guard.twi.twi_cntr.modify(|_r, w| {
            w.m_stp().variant(true);
            w.a_ack().variant(false);
            w
        });
        if let Some(err) = guard.data.err.take() {
            Err(err)
        } else {
            match core::mem::replace(&mut guard.data.op, Op::None) {
                Op::Read { buf, .. } => Ok(buf),
                _ => unreachable!(),
            }
        }

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
    }
}

impl Twi {
    #[must_use]
    fn lock<'a>(&'a self, twi: &'a twi::RegisterBlock) -> TwiDataGuard<'a> {
        kernel::trace::info!("twi locked");
        // disable TWI interrupts while holding the guard.
        twi.twi_cntr.modify(|_r, w| w.int_en().low());
        let data = unsafe { &mut *(self.data.get()) };
        TwiDataGuard { data, twi }
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

impl TwiData {
    fn advance_isr(&mut self, twi: &twi::RegisterBlock, waiter: &WaitCell) {
        use status::*;

        // for _ in 0..10_000 {
        //     core::hint::spin_loop();
        // }
        let status: u8 = twi.twi_stat.read().sta().bits();

        tracing::info!(status = ?format_args!("{status:#x}"), state = ?self.state, "TWI interrupt");

        self.state = match self.state {
            // State::Idle => {
            //     // TODO: send a STOP?
            //     State::Idle
            // }
            State::WaitForStart(addr)
                if status == START_TRANSMITTED || status == REPEATED_START_TRANSMITTED =>
            {
                let bits = {
                    // lowest bit is 1 if reading, 0 if writing.
                    let dir = match self.op {
                        Op::Read { .. } => 0b1,
                        _ => 0b0,
                    };
                    match addr {
                        Addr::SevenBit(addr) => ((addr & 0x7f) << 1) | dir,
                        Addr::TenBit(_) => todo!("eliza: implement ten bit addrs"),
                    }
                };

                // send the address
                twi.twi_data.write(|w| w.data().variant(bits));
                // for _ in 0..5 * 1000 {
                //     core::hint::spin_loop();
                // }

                // The data sheet specifically says that we don't have to do
                // this, but it seems to be lying...
                twi.twi_cntr.modify(|_r, w| w.m_sta().clear_bit());
                State::WaitForAddr1Ack(addr)
            }
            State::WaitForStart(addr) => {
                twi.twi_cntr
                    .modify(|_r, w: &mut twi::twi_cntr::W| w.m_sta().variant(true));

                State::WaitForStart(addr)
            }
            // Sometimes we get the interrupt with this bit set multiple times.
            State::WaitForAddr1Ack(addr) if status == REPEATED_START_TRANSMITTED => {
                State::WaitForAddr1Ack(addr)
            }
            State::WaitForAddr1Ack(Addr::SevenBit(_)) if status == ADDR1_WRITE_ACKED =>
            // TODO(eliza): handle 10 bit addr...
            {
                if let Op::Write { buf, ref mut pos } = &mut self.op {
                    // send the first byte of data
                    twi.twi_data.write(|w| w.data().variant(buf.as_slice()[0]));
                    *pos += 1;
                    State::WaitForAck
                } else {
                    unreachable!(
                        "if we sent an address with a write bit, we should be in a write state"
                    )
                }
            }
            State::WaitForAddr1Ack(Addr::SevenBit(_)) if status == ADDR1_READ_ACKED =>
            // TODO(eliza): handle 10 bit addr...
            {
                match self.op {
                    Op::Read { .. } => State::WaitForData,
                    Op::None =>                     unreachable!(
                        "if we sent an address with a read bit, we should be in a read state (was None)"
                    ),
                    Op::Write { .. } => unreachable!(
                        "if we sent an address with a read bit, we should be in a read state (was Write)"
                    ),
                }
            }
            State::WaitForData if status == RX_DATA_ACKED => {
                match &mut self.op {
                    Op::Read { buf, amt, read } => {
                        let data = twi.twi_data.read().data().bits();
                        buf.copy_from_slice(&[data]);
                        *read += 1;
                        if read < amt {
                            State::WaitForData
                        } else {
                            twi.twi_cntr.modify(|_r, w| w.int_en().low());
                            waiter.wake();
                            // twi.twi_cntr.modify(|_r, w| w.m_stp().variant(true));
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
                match &mut self.op {
                    Op::Write { buf, pos } => {
                        if *pos < buf.as_slice().len() {
                            // send the next byte of data
                            twi.twi_data
                                .write(|w| w.data().variant(buf.as_slice()[*pos]));
                            *pos += 1;
                            State::WaitForAck
                        } else {
                            twi.twi_cntr.modify(|_r, w| w.int_en().low());
                            waiter.wake();
                            // twi.twi_cntr.modify(|_r, w| w.m_stp().variant(true));
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
                let err = status::error(status);
                panic!("TWI0 error: {err:?}, {status:#x}, {:?}", self.state);
                // kernel::trace::warn!(?err, ?status, state = ?self.state, "TWI0 error");
                self.err = Some(err);
                twi.twi_cntr.modify(|_r, w| {
                    w.int_en().low();
                    w.m_stp().variant(true);
                    w
                });
                waiter.wake();
                State::Idle
            }
        };
        twi.twi_cntr.modify(|_r, w| w.int_flag().clear_bit());
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
