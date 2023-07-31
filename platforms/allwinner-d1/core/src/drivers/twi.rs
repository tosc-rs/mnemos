//! Drivers for the Allwinner D1's I²C/TWI peripherals.
//!
//! This module contains an implementation of a driver for controlling the
//! I²C hardware on the D1, which the D1 manual calls a TWI
//! (Two-Wire Interface), likely due to I²C being a trademark of
//! Phillips Semiconductor. The [`I2c0`] type in this module implements the
//! [`I2cService`] trait using the TWI hardware mapped to the `I2C0` pins on the
//! board's 40-pin Raspberry Pi header.
//!
//! The D1 has four separate TWI controllers, [`TWI0`], [`TWI1`], [`TWI2`], and
//! [`TWI3`]. The pin mapping for the 40-pin Pi header `I2C0` pins differs
//! between the MangoPi MQ Pro and the Lichee RV Dock. On the MQ Pro, [`TWI0`]
//! is used, with `SCL` on pin `PG12` and `SDA` on pin `PG13`. On the Lichee
//! RV Dock, [`TWI2`] is used instead, with `SCL` on pin `PB0` and `SDA` on
//! pin `PB1`. Separate constructors for the [`I2c0`] type,
//! [`I2c0::mq_pro`] and [`I2c0::lichee_rv_dock`], are provided to
//! configure the appropriate pins for each board.
//!
//! ## Implementation Notes
//!
//! The TWI hardware can be used in one of two modes: "TWI engine" mode, where
//! individual bytes are written/read from the I²C bus in an
//! interrupt handler, and "TWI driver" mode, where the TWI hardware can operate
//! at the level of I²C register read/writes using a DMA buffer. This
//! module currently only implements a driver for the "TWI engine" mode, since
//! it can model all forms of I²C operations. In the future, we will
//! likely want to opportunistically use the offload capabilities of the TWI
//! driver when the I²C transaction has the correct shape for
//! offloading, but this branch just implements the simpler TWI engine mode.
//!
//! The TWI hardware is a bit difficult to use correctly, so implementing this
//! was a bit of a struggle. In particular, it turns out that the generation of
//! I²C clock pulses occurs when the
//! [`TWI_CNTR`](d1_pac::twi::TWI_CNTR) register, which controls the TWI, is
//! written to.
//!
//! The driver works by sharing state between a driver task and an ISR, since
//! the TWI engine mode is interrupt driven. The shared state is "locked" by
//! disabling TWI interrupts temporarily while the driver task is writing to the
//! shared state, and resuming interrupts when a write to the shared state
//! completes. In theory, this driver is also safe for use on multi-core
//! hardware, since a single daemon task is responsible for all the non-ISR
//! writes to this shared state, but this doesn't actually matter, because the
//! D1 is an inherently single-core CPU.
//!
//! I believe that the I²C controller used in the D1 is from the
//! Marvell MV64xxx family, although I'm not sure which one in particular. Linux
//! has a driver for this device, which can be found [here][linux-driver].
//!
//! [linux-driver]: https://github.com/torvalds/linux/blob/995b406c7e972fab181a4bb57f3b95e59b8e5bf3/drivers/i2c/busses/i2c-mv64xxx.c
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    task::{Poll, Waker},
};

use crate::ccu::Ccu;
use d1_pac::{twi, Interrupt, GPIO, TWI0, TWI1, TWI2, TWI3};
use kernel::{
    comms::kchannel::{KChannel, KConsumer},
    embedded_hal_async::i2c::{ErrorKind, NoAcknowledgeSource},
    mnemos_alloc::containers::FixedVec,
    registry,
    services::i2c::{
        messages::{OpKind, Transfer},
        Addr, I2cService, Transaction,
    },
    tracing, Kernel,
};

/// A TWI mapped to the Raspberry Pi header's I²C0 pins.
pub struct I2c0 {
    isr: &'static IsrData,
    twi: &'static twi::RegisterBlock,
    /// Which TWI does this TWI Engine use?
    int: (Interrupt, fn()),
}

/// Data used by a TWI interrupt.
struct IsrData {
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

static I2C0_ISR: IsrData = IsrData {
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
        amt: usize,
        end: bool,
    },
    None,
}

/// TWI state machine
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[allow(dead_code)] // TODO(eliza): implement 10-bit addresses
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

// === impl I2c0 ===

impl I2c0 {
    /// Initialize a TWI for the MangoPi MQ Pro's Pi header I²C0
    /// pins. This configures TWI0 in TWI engine mode, with the MangoPi MQ Pro pin
    /// mappings.
    ///
    /// # Safety
    ///
    /// - The TWI register block must not be concurrently written to.
    /// - This function should be called only while running on a MangoPi MQ Pro
    ///   board.
    pub unsafe fn mq_pro(mut twi: TWI0, ccu: &mut Ccu, gpio: &mut GPIO) -> Self {
        // Step 1: Configure GPIO pin mappings.
        gpio.pg_cfg1.modify(|_r, w| {
            // on the Mango Pi MQ Pro, the pi header's I2C0 pins are mapped to
            // TWI0 on PG12 and PG13:
            // https://mangopi.org/_media/mq-pro-sch-v12.pdf
            w.pg12_select().twi0_sck();
            w.pg13_select().twi0_sda();
            w
        });

        ccu.disable_module(&mut twi);

        ccu.enable_module(&mut twi);

        Self::init(
            unsafe { &*TWI0::ptr() },
            Interrupt::TWI0,
            Self::handle_twi0_interrupt,
        )
    }

    /// Initialize a TWI for the Lichee RV Dock's Pi header I²C0
    /// pins. This configures TWI2 in TWI engine mode, with the Lichee RV pin
    /// mappings.
    ///
    /// # Safety
    ///
    /// - The TWI register block must not be concurrently written to.
    /// - This function should be called only while running on a Lichee RV
    ///   board.
    pub unsafe fn lichee_rv_dock(mut twi: TWI2, ccu: &mut Ccu, gpio: &mut GPIO) -> Self {
        // Step 1: Configure GPIO pin mappings.
        gpio.pb_cfg0.modify(|_r, w| {
            // on the Lichee RV Dock, the Pi header's I2C0 corresponds to TWI2, not
            // TWI0 as on the MQ Pro.
            // I2C0 SDA is mapped to TWI2 PB1, and I2C0 SCL is mapped to TWI2 PB0:
            // https://dl.sipeed.com/fileList/LICHEE/D1/Lichee_RV-Dock/2_Schematic/Lichee_RV_DOCK_3516(Schematic).pdf
            w.pb0_select().twi2_sck();
            w.pb1_select().twi2_sda();
            w
        });

        ccu.disable_module(&mut twi);

        ccu.enable_module(&mut twi);

        Self::init(
            unsafe { &*TWI2::ptr() },
            Interrupt::TWI2,
            Self::handle_twi2_interrupt,
        )
    }

    /// Returns the interrupt and ISR for this TWI.
    pub fn interrupt(&self) -> (Interrupt, fn()) {
        self.int
    }

    /// Handle a TWI 0 interrupt on the I2C0 pins.
    fn handle_twi0_interrupt() {
        let _isr = kernel::isr::Isr::enter();
        let twi = unsafe { &*TWI0::PTR };
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*I2C0_ISR.data.get())
        };

        data.advance_isr(twi, 0);
    }

    /// Handle a TWI 1 interrupt on the I2C0 pins.
    #[allow(dead_code)] // may be used if we ever have a board that maps TWI1 to I2C0...
    fn handle_twi1_interrupt() {
        let _isr = kernel::isr::Isr::enter();
        let twi = unsafe { &*TWI1::PTR };
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*I2C0_ISR.data.get())
        };

        data.advance_isr(twi, 1);
    }

    /// Handle a TWI 2 interrupt on the I2C0 pins.
    fn handle_twi2_interrupt() {
        let _isr = kernel::isr::Isr::enter();
        let twi = unsafe { &*TWI2::PTR };
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*I2C0_ISR.data.get())
        };

        data.advance_isr(twi, 2);
    }

    /// Handle a TWI 3 interrupt on the I2C0 pins.
    #[allow(dead_code)] // may be used if we ever have a board that maps TWI1 to I2C0...
    fn handle_twi3_interrupt() {
        let _isr = kernel::isr::Isr::enter();
        let twi = unsafe { &*TWI3::PTR };
        let data = unsafe {
            // safety: it's okay to do this because this function can only be
            // called from inside the ISR.
            &mut (*I2C0_ISR.data.get())
        };

        data.advance_isr(twi, 3);
    }

    /// This assumes the GPIO pin mappings are already configured.
    unsafe fn init(twi: &'static twi::RegisterBlock, int: Interrupt, isr: fn()) -> Self {
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
            w.m_stp().set_bit();
            w
        });

        // we only want to be the bus controller, so zero our address
        twi.twi_addr.write(|w| w.sla().variant(0));
        twi.twi_xaddr.write(|w| w.slax().variant(0));

        Self {
            twi,
            isr: &I2C0_ISR,
            int: (int, isr),
        }
    }

    pub async fn register(self, kernel: &'static Kernel, queued: usize) -> Result<(), ()> {
        let (tx, rx) = KChannel::new_async(queued).await.split();

        kernel.spawn(self.run(rx)).await;
        tracing::info!("TWI driver task spawned");
        kernel
            .with_registry(move |reg| reg.register_konly::<I2cService>(&tx).map_err(drop))
            .await?;

        Ok(())
    }

    #[tracing::instrument(name = "TWI", level = tracing::Level::INFO, skip(self, rx))]
    async fn run(self, rx: KConsumer<registry::Message<I2cService>>) {
        tracing::info!("starting TWI driver task");
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
        tracing::trace!("starting I2C transaction");
        let mut guard = self.isr.lock(self.twi);

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
                OpKind::Read => {
                    // setup read op
                    tracing::debug!("reading {len} bytes");
                    TwiOp::Read {
                        buf,
                        len,
                        amt: 0,
                        end,
                    }
                }
                OpKind::Write => {
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
            tracing::trace!(?dir, "TWI operation completed");
            let res = if let Some(error) = guard.data.err.take() {
                tracing::warn!(?error, ?dir, "TWI error");
                Err(error)
            } else {
                match core::mem::replace(&mut guard.data.op, TwiOp::None) {
                    TwiOp::Read { buf, .. } => {
                        debug_assert_eq!(dir, OpKind::Read);
                        Ok(buf)
                    }
                    TwiOp::Write { buf, .. } => {
                        debug_assert_eq!(dir, OpKind::Write);
                        Ok(buf)
                    }
                    _ => unreachable!(),
                }
            };
            if rsp.send(res).is_err() {
                tracing::trace!("I2C transaction handle dropped");
                break;
            }
        }
        // transaction ended!
        tracing::trace!("I2C transaction ended");
    }
}

impl IsrData {
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
                // we have to set M_STA and A_ACK as part of the same write that
                // sets the INT_EN bit, or else we will potentially do something
                // weird if we do two separate TWI_CNTR writes. setting all of
                // these now, atomically, avoids weird cases where we send a
                // START for some random address, as far as i can tell.
                w.m_sta().set_bit();
                w.int_en().high();
                w.bus_en().respond();
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
        self.data
    }
}

impl TwiData {
    fn advance_isr(&mut self, twi: &twi::RegisterBlock, num: u8) {
        let status = {
            let byte = twi.twi_stat.read().sta().bits();
            match Status::try_from(byte) {
                Ok(status) => status,
                Err(error) => {
                    tracing::error!(status = ?format_args!("{byte:#x}"), %error, twi = num, "TWI{num} status invalid");
                    return;
                }
            }
        };
        let mut needs_wake = false;
        tracing::trace!(?status, state = ?self.state, twi = num, "TWI{num} interrupt");
        twi.twi_cntr.modify(|_cntr_r, cntr_w| {
            self.state = match (self.state, status)  {
                (State::Idle, _) => {
                    cntr_w.m_stp().set_bit();
                    State::Idle
                }
                (State::WaitForStart(addr), Status::StartTransmitted) |
                (State::WaitForRestart(addr), Status::RepeatedStartTransmitted) => {
                    let bits = {
                        // lowest bit is 1 if reading, 0 if writing.
                        let dir = match self.op {
                            TwiOp::Read { .. } => 0b1,
                            _ => 0b0,
                        };
                        let addr = match addr {
                            Addr::SevenBit(addr) => (addr & 0x7f) << 1,
                            Addr::TenBit(addr) => 0xf0 | ((addr & 0x300) >> 7) as u8,
                        };
                        addr | dir
                    };
                    // send the address
                    twi.twi_data.write(|w| w.data().variant(bits));
                    State::WaitForAddr1Ack(addr)
                }
                // Write address byte 2.
                (State::WaitForAddr1Ack(Addr::TenBit(addr)), Status::Addr1ReadAcked) |
                (State::WaitForAddr1Ack(Addr::TenBit(addr)), Status::Addr1WriteAcked) => {
                    twi.twi_data.write(|w| w.data().variant(addr as u8));
                    State::WaitForAddr2Ack(Addr::TenBit(addr))
                }
                // Last address byte was ACKed (write), so we can start sending data.
                (State::WaitForAddr1Ack(addr @ Addr::SevenBit(_)), Status::Addr1WriteAcked) |
                (State::WaitForAddr2Ack(addr @ Addr::TenBit(_)), Status::Addr2WriteAcked) =>
                {
                    match &mut self.op {
                        TwiOp::Write { buf, ref mut pos, .. } => {
                            // send the first byte of data
                            let byte = buf.as_slice()[0];
                            tracing::trace!(twi = num, data = ?format_args!("{byte:#x}"), "TWI{num} write data");
                            twi.twi_data.write(|w| w.data().variant(byte));
                            *pos += 1;
                            State::WaitForAck(addr)
                        },
                        TwiOp::Read { .. } => unreachable!(
                            "if we sent an address with a write bit, we should be in a write state (was Read)"
                        ),
                        TwiOp::None => unreachable!(
                            "if we sent an address with a write bit, we should be in a write state (was None)"
                        ),
                    }
                }

                // Last address byte was ACKed (read), so we can start reading data.
                (State::WaitForAddr1Ack(addr @ Addr::SevenBit(_)), Status::Addr1ReadAcked) |
                (State::WaitForAddr2Ack(addr @ Addr::TenBit(_)), Status::Addr2ReadAcked) =>
                // TODO(eliza): handle 10 bit addr...
                {
                    match self.op {
                        TwiOp::Read { len, .. } => {
                            if len == 1 {
                                // if we are reading a single byte, clear the A_ACK
                                // flag so that we don't ACK the byte.
                                cntr_w.a_ack().clear_bit();
                            } else {
                                // otherwise, we'll want to ACK all but the
                                // final byte.
                                cntr_w.a_ack().set_bit();
                            }
                            State::WaitForData(addr)
                        }
                        TwiOp::None => unreachable!(
                            "if we sent an address with a read bit, we should be in a read state (was None)"
                        ),
                        TwiOp::Write { .. } => unreachable!(
                            "if we sent an address with a read bit, we should be in a read state (was Write)"
                        ),
                    }
                }
                (State::WaitForData(addr), Status::RxDataAcked) | (State::WaitForData(addr), Status::RxDataNacked) => {
                    match &mut self.op {
                        &mut TwiOp::Read { ref mut buf, len, amt: ref mut read, end } => {
                            let data = twi.twi_data.read().data().bits();
                            buf.try_push(data).expect("read buf should have space for data");
                            *read += 1;
                            let remaining = len - *read;
                            tracing::trace!(
                                twi = num,
                                data = ?format_args!("{data:#x}"),
                                end,
                                remaining,
                                "TWI{num} read data",
                            );

                            if remaining <= 1 {
                                // We have one byte left to read, so stop ACKing.
                                cntr_w.a_ack().clear_bit();
                            } else {
                                // We have more than one byte remaining, so
                                // continue ACKing.
                                cntr_w.a_ack().set_bit();
                            }

                            if remaining > 0 {
                                State::WaitForData(addr)
                            } else {
                                needs_wake = true;
                                // if this is the last operation in the
                                // transaction, send a STOP.
                                if end {
                                    tracing::trace!(twi = num, "TWI{num} send STOP");
                                    cntr_w.m_stp().set_bit();
                                    State::Idle
                                } else {
                                    // otherwise, send a repeated START for the
                                    // next operation.
                                    tracing::trace!(twi = num, "TWI{num} send repeated START");
                                    cntr_w.m_sta().set_bit();
                                    State::WaitForRestart(addr)
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                (State::WaitForAck(addr), Status::TxDataAcked) => {
                    match &mut self.op {
                        &mut TwiOp::Write { ref mut buf, ref mut pos, len, end } => {
                            if *pos == len {
                                needs_wake = true;
                                // Send a repeated START for the read portion of
                                // the transaction.
                                if end {
                                    tracing::trace!(twi = num, "TWI{num} send STOP");
                                    cntr_w.m_stp().set_bit();
                                    State::Idle
                                } else {
                                    // otherwise, send a repeated START for the
                                    // next operation.
                                    cntr_w.m_sta().set_bit();
                                    tracing::trace!(twi = num, "TWI{num} send repeated START");
                                    State::WaitForRestart(addr)
                                }
                            } else {
                                // Send the next byte of data
                                let byte = buf.as_slice()[*pos];
                                tracing::trace!(
                                    twi = num,
                                    remaining = len - *pos,
                                    data = ?format_args!("{byte:#x}"),
                                    "TWI{num} write data"
                                );

                                twi.twi_data.write(|w| w.data().variant(byte));

                                *pos += 1;
                                State::WaitForAck(addr)
                            }
                        }
                        _ => unimplemented!(),
                    }
                }
                (_, status) => {
                    let error: ErrorKind = status.into_error();
                    tracing::warn!(?error, ?status, state = ?self.state, twi = num, "TWI{num} error");
                    self.err = Some(error);
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

unsafe impl Sync for IsrData {}

// TODO(eliza): this ought to go in `mycelium-bitfield` eventually
macro_rules! enum_try_from {
    (
        $(#[$meta:meta])* $vis:vis enum $name:ident<$repr:ty> {
            $(
                $(#[$var_meta:meta])*
                $variant:ident = $value:expr
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr($repr)]
        $vis enum $name {
            $(
                $(#[$var_meta])*
                $variant = $value
            ),*
        }


        impl core::convert::TryFrom<$repr> for $name {
            type Error = &'static str;

            fn try_from(value: $repr) -> Result<Self, Self::Error> {
                match value {
                    $(
                        $value => Ok(Self::$variant),
                    )*
                    _ => Err(concat!(
                        "invalid value for ",
                        stringify!($name),
                        ": expected one of [",
                        $(
                            stringify!($value),
                            ", ",
                        )* "]")
                    ),
                }
            }
        }
    };
}

enum_try_from! {
    /// Values of the `TWI_STAT` register.
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    enum Status<u8> {
        /// 0x00: Bus error
        BusError = 0x00,
        /// 0x08: START condition transmitted
        StartTransmitted = 0x08,
        /// Ox10: Repeated START condition transmitted
        RepeatedStartTransmitted = 0x10,
        /// 0x18: Address + Write bit transmitted, ACK received
        Addr1WriteAcked = 0x18,
        /// 0x20: Address + Write bit transmitted, NACK received
        Addr1WriteNacked = 0x20,
        /// 0x28: Data byte transmitted in controller mode, ACK received
        TxDataAcked = 0x28,
        /// 0x30: Data byte transmitted in controller mode, ACK not received
        TxDataNacked = 0x30,
        /// 0x38: Arbitration lost in address or data byte
        ArbitrationLost = 0x38,
        /// 0x40: Address + Read bit transmitted, ACK received
        Addr1ReadAcked = 0x40,
        /// 0x48: Address + Read bit transmitted, ACK not received
        Addr1ReadNacked = 0x48,
        /// 0x50: Data byte received in controller mode, ACK transmitted
        RxDataAcked = 0x50,
        /// 0x58: Data byte received in controller mode, no ACK transmitted
        RxDataNacked = 0x58,
        /// 0x60: Target address and write bit received, ACK transmitted
        TargetAddrWriteAcked = 0x60,
        /// 0x68: Arbitration lost in the address as controller, target address
        /// + Write bit recieved, ACK transmitted
        ArbitrationLostTargetWrite = 0x68,
        /// 0x70: General call address received, ACK transmitted
        GeneralCall = 0x70,
        /// 0x78: Arbitration lost in the address as controller, General Call
        /// address transmitted, ACK received
        ArbitrationLostGeneralCall = 0x78,
        /// 0x80: Data byte recieved after target address received, ACK
        /// transmitted.
        TargetRxDataAcked = 0x80,
        /// 0x80: Data byte recieved after target address received, no ACK
        /// transmitted.
        TargetRxDataNacked = 0x88,
        /// 0x90: Data byte received after General Call received, ACK
        /// transmitted.
        GeneralCallRxDataAcked = 0x90,
        /// 0x98: Data byte received after General Call received, no ACK
        /// transmitted
        GeneralCallRxDataNacked = 0x98,
        /// 0xA0: STOP or repeated START condition received in target mode
        TargetStopOrRepeatedStart = 0xA0,
        /// 0xA8: Target address + Read bit received, ACK transmitted
        TargetAddrReadAcked = 0xA8,
        /// 0xB0: Arbitration lost in address as controller, target address +
        /// Read bit received, ACK transmitted
        ArbitrationLostTargetRead = 0xB0,
        /// 0xB8: Data byte transmitted in target mode, ACK received
        TargetTxDataAcked = 0xB8,
        /// 0xC0: Data byte transmitted in target mode, ACK not received
        TargetTxDataNacked = 0xC0,
        /// 0xC8: Last data byte transmitted in target mode, ACK received
        TargetTxLastDataAcked = 0xC8,
        /// 0xD0: Second Address byte + Write bit transmitted, ACK received
        Addr2WriteAcked = 0xD0,
        /// 0xD8: Second Address byte + Write bit transmitted, ACK not received
        Addr2WriteNacked = 0xD8,
        /// 0xE0: Address 2 + Read bit received, ACK transmitted
        ///
        /// Note that the D1 manual neglects to mention this state (and the
        /// corresponding NACK state), but [it's in the Linux driver][linux], so
        /// I presume it's real.alloc
        ///
        /// [linux]:
        ///     https://github.com/torvalds/linux/blob/46670259519f4ee4ab378dc014798aabe77c5057/drivers/i2c/busses/i2c-mv64xxx.c#L57
        Addr2ReadAcked = 0xE0,
        /// 0xE8: Address 2 + Read bit received, ACK transmitted
        ///
        /// Note that the D1 manual neglects to mention this state (and the
        /// corresponding NACK state), but [it's in the Linux driver][linux], so
        /// I presume it's real.alloc
        ///
        /// [linux]:
        ///     https://github.com/torvalds/linux/blob/46670259519f4ee4ab378dc014798aabe77c5057/drivers/i2c/busses/i2c-mv64xxx.c#L58
        Addr2ReadNacked = 0xE8,
        /// 0xF8: No relevant status information, `INT_FLAG` = 0
        None = 0xF8,
    }
}

impl Status {
    fn into_error(self) -> ErrorKind {
        match self {
            // Arbitration Lost errors
            Self::ArbitrationLost
            | Self::ArbitrationLostGeneralCall
            | Self::ArbitrationLostTargetRead
            | Self::ArbitrationLostTargetWrite => ErrorKind::ArbitrationLoss,
            Self::BusError => ErrorKind::Bus,
            // Address NACKed errors
            Self::Addr1WriteNacked
            | Self::Addr1ReadNacked
            | Self::Addr2WriteNacked
            | Self::Addr2ReadNacked => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
            // Send data NACKed errors
            Self::TxDataNacked | Self::TargetTxDataNacked => {
                ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data)
            }
            _ => ErrorKind::Other,
        }
    }
}
