//! I<sup>2</sup>C Driver Service
//!
//! This module contains a service definition for drivers for the I<sup>2</sup>C
//! bus.
//!
//! ## About I<sup>2</sup>C
//!
//! I<sup>2</sup>C, according to the [RP2040 datasheet], is "an ubiquitous
//! serial bus first described in the Dead Sea Scrolls, and later used by
//! Philips Semiconductor". It's a two-wire, multi-drop bus, allowing multiple
//! devices to be connected to a single clock and data line.
//!
//! Unlike SPI, this is a "real protocol", and not just a sort of shared
//! hallucination about the meanings of certain wires. That means it has
//! _rules_. Some of these rules are relevant to users of this module.
//!
//! ## Usage
//!
//! Users of the I<sup>2</sup>C bus will primarily interact with this module
//! using the [`I2cClient`] type, which implements a client for the
//! [`I2cService`] service. This client type can be used to perform read and
//! write operations on the I<sup>2</sup>C bus. A new client can be acquired
//! using [`I2cClient::from_registry`].
//!
//! Once an [`I2cClient`] has been obtained, it can be used to perform
//! I<sup>2</sup>C operations. Two interfaces are available: an
//! [implementation][impl-i2c] of the [`embedded_hal_async::i2c::I2c`] trait,
//! and a lower-level interface using the [`I2cClient::start_transaction`]
//! method. In general, the [`embedded_hal_async::i2c::I2c`] trait is the
//! recommended interface.
//!
//! The lower-level interface allows reusing the same heap-allocated buffer for
//! multiple I<sup>2</sup>C bus transactions. It also provides the ability to
//! interleave other code between the write and read operations of an
//! I<sup>2</sup>C transaction without sending a STOP condition. If either of
//! these are necessary, the [`Transaction`] interface may be preferred over the
//! [`embedded_hal_async`] interface.
//!
//! [RP2040 datasheet]: https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf
//! [impl-i2c]: I2cClient#impl-I2c<u8>-for-I2cClient
#![warn(missing_docs)]
use self::messages::*;
use crate::{
    comms::{
        kchannel::{KChannel, KConsumer, KProducer},
        oneshot::{self, Reusable},
    },
    mnemos_alloc::containers::FixedVec,
    registry::{known_uuids, Envelope, KernelHandle, RegisteredDriver},
    Kernel,
};
use core::{convert::Infallible, fmt, time::Duration};
use embedded_hal_async::i2c;
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// [Service](crate::services) definition for I<sup>2</sup>C bus drivers.
///
/// See the [module-level documentation](crate::services::i2c) for details on
/// using this service.
pub struct I2cService;

impl RegisteredDriver for I2cService {
    type Request = StartTransaction;
    type Response = Transaction;
    type Error = core::convert::Infallible;

    const UUID: Uuid = known_uuids::kernel::I2C;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

/// I<sup>2</sup>C bus address, in either 7-bit or 10-bit address format.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Addr {
    /// A 7-bit I<sup>2</sup>C address.
    SevenBit(u8),
    /// A 10-bit extended I<sup>2</sup>C address.
    TenBit(u16),
}

/// A transaction on the I<sup>2</sup>C bus.
pub struct Transaction {
    addr: Addr,
    tx: KProducer<Transfer>,
    rsp_rx: Reusable<Result<FixedVec<u8>, i2c::ErrorKind>>,
}

/// Errors returned by the `I2cService`
#[derive(Debug)]
pub struct I2cError {
    addr: Addr,
    kind: ErrorKind,
    dir: OpKind,
}

/// Messages used to communicate with an [`I2cService`] implementation.
pub mod messages {
    use super::*;

    /// Message sent to an [`I2cService`] by an [`I2cClient`] in order to start a
    /// new bus [`Transaction`]
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub struct StartTransaction {
        /// The address of the target device for this transaction.
        pub addr: Addr,
        pub(super) capacity: usize,
    }

    pub struct Transfer {
        pub buf: FixedVec<u8>,
        pub len: usize,
        pub end: bool,
        pub dir: OpKind,
        pub rsp: oneshot::Sender<Result<FixedVec<u8>, i2c::ErrorKind>>,
    }

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum OpKind {
        Read,
        Write,
    }

    pub struct ReadOp {
        pub buf: FixedVec<u8>,
        pub len: usize,
        pub end: bool,
    }

    pub struct WriteOp {
        pub buf: FixedVec<u8>,
        pub len: usize,
        pub end: bool,
    }
}

#[derive(Debug)]
enum ErrorKind {
    I2c(i2c::ErrorKind),
    NoDriver,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////
pub struct I2cClient {
    handle: KernelHandle<I2cService>,
    reply: Reusable<Envelope<Result<Transaction, Infallible>>>,
}

impl I2cClient {
    /// Obtain an `I2cClient`
    ///
    /// If the [`I2cService`] hasn't been registered yet, we will retry until it has been
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match I2cClient::from_registry_no_retry(kernel).await {
                Some(port) => return port,
                None => {
                    // I2C probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain an `I2cClient`
    ///
    /// Does NOT attempt to get an [`I2cService`] handle more than once.
    ///
    /// Prefer [`I2cClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let handle = kernel.with_registry(|reg| reg.get::<I2cService>()).await?;

        Some(I2cClient {
            handle,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn start_transaction(&mut self, addr: Addr) -> Transaction {
        let resp = self
            .handle
            .request_oneshot(StartTransaction { addr, capacity: 2 }, &self.reply)
            .await
            .unwrap();
        resp.body.expect("transaction should be created")
    }
}

impl i2c::ErrorType for I2cClient {
    type Error = I2cError;
}

impl i2c::I2c<i2c::SevenBitAddress> for I2cClient {
    async fn transaction(
        &mut self,
        address: i2c::SevenBitAddress,
        operations: &mut [i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        let mut buf = {
            // determine the maximum size operation to allocate a buffer for the
            // transaction.
            let len = operations
                .iter()
                .map(|op| match op {
                    i2c::Operation::Read(buf) => buf.len(),
                    i2c::Operation::Write(buf) => buf.len(),
                })
                .max();
            FixedVec::new(len.unwrap_or(0)).await
        };
        let mut txn = self.start_transaction(Addr::SevenBit(address)).await;
        let n_ops = operations.len();
        for (n, op) in operations.iter_mut().enumerate() {
            buf.clear();
            let end = n == n_ops - 1;
            match op {
                i2c::Operation::Read(dest) => {
                    let len = dest.len();
                    let read = txn.read(buf, len, end).await?;
                    dest.copy_from_slice(read.as_slice());
                    buf = read;
                }
                i2c::Operation::Write(src) => {
                    let len = src.len();
                    buf.try_extend_from_slice(src)
                        .expect("we should have pre-allocated a large enough buffer!");
                    let write = txn.write(buf, len, end).await?;
                    buf = write;
                }
            }
        }
        // TODO(eliza): save the buffer so that we can use it for future transactions?
        Ok(())
    }
}

// === impl Transaction ===

impl Transaction {
    pub async fn new(
        StartTransaction { addr, capacity }: StartTransaction,
    ) -> (Self, KConsumer<Transfer>) {
        let (tx, rx) = KChannel::new_async(capacity).await.split();
        let rsp_rx = Reusable::new_async().await;
        let txn = Transaction { addr, rsp_rx, tx };
        (txn, rx)
    }

    /// Read `len` bytes from the I<sup>2</sup>C bus into `buf`.
    ///
    /// Note that, rather than always filling the entire buffer, this method
    /// takes a `len` argument which specifies the number of bytes to read. This
    /// is intended to allow callers to reuse the same [`FixedVec`] for multiple
    /// `read` and [`write`] operations.
    ///
    /// # Arguments
    ///
    /// - `buf`: a [`FixedVec`] buffer into which bytes read from the
    ///   I<sup>2</sup>C bus will be written
    /// - `len`: the number of bytes to read. This must be less than or equal to
    ///   `buf.len()`.
    /// - `end`: whether or not to end the transaction. If this is `true`, a
    ///   `STOP` condition will be sent on the bus once `len` bytes have been
    ///   read. If this is `false`, a repeated `START` condition will be sent on
    ///   the bus once `len` bytes have been read.
    ///
    /// # Panics
    ///
    /// - If `len` is greater `buf.capacity() - `buf.len()`.
    pub async fn read(
        &mut self,
        buf: FixedVec<u8>,
        len: usize,
        end: bool,
    ) -> Result<FixedVec<u8>, I2cError> {
        assert!(
            len <= buf.capacity() - buf.len(),
            "read length ({len}) exceeds remaining buffer capacity ({})",
            buf.capacity() - buf.len()
        );
        self.xfer(buf, len, end, OpKind::Read).await
    }

    pub async fn write(
        &mut self,
        buf: FixedVec<u8>,
        len: usize,
        end: bool,
    ) -> Result<FixedVec<u8>, I2cError> {
        self.xfer(buf, len, end, OpKind::Write).await
    }

    async fn xfer(
        &mut self,
        buf: FixedVec<u8>,
        len: usize,
        end: bool,
        dir: OpKind,
    ) -> Result<FixedVec<u8>, I2cError> {
        let rsp = self
            .rsp_rx
            .sender()
            .await
            .expect("sender should not be in use");
        let xfer = Transfer {
            buf,
            len,
            end,
            rsp,
            dir,
        };
        self.tx
            .enqueue_async(xfer)
            .await
            .map_err(I2cError::mk_no_driver(self.addr, dir))?;
        self.rsp_rx
            .receive()
            .await
            .map_err(I2cError::mk_no_driver(self.addr, dir))?
            .map_err(I2cError::mk(self.addr, dir))
    }
}

// TODO(eliza): if we properly implement close-on-drop behavior for KProducers,
// we can remove this...
impl Drop for Transaction {
    fn drop(&mut self) {
        self.tx.close();
    }
}

// === impl Addr ===

impl fmt::Debug for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Addr::SevenBit(addr) => write!(f, "SevenBit(0x{addr:02x})"),
            Addr::TenBit(addr) => write!(f, "TenBit(0x{addr:04x})"),
        }
    }
}

impl Addr {
    /// Returns the low 7 bits of this address.
    #[must_use]
    pub fn low_bits(self) -> u8 {
        match self {
            Self::SevenBit(bits) => bits,
            Self::TenBit(bits) => (bits & 0b0111_1111) as u8,
        }
    }
}

// === impl I2cError ===

impl I2cError {
    pub fn new(addr: Addr, kind: i2c::ErrorKind, dir: OpKind) -> Self {
        Self {
            addr,
            kind: ErrorKind::I2c(kind),
            dir,
        }
    }

    #[inline]
    #[must_use]
    pub fn direction(&self) -> OpKind {
        self.dir
    }

    fn mk(addr: Addr, dir: OpKind) -> impl Fn(i2c::ErrorKind) -> Self {
        move |kind| Self {
            addr,
            kind: ErrorKind::I2c(kind),
            dir,
        }
    }

    fn mk_no_driver<E>(addr: Addr, dir: OpKind) -> impl Fn(E) -> Self {
        move |_| Self {
            addr,
            dir,
            kind: ErrorKind::NoDriver,
        }
    }
}

impl fmt::Display for I2cError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { addr, kind, dir } = self;
        let verb = match dir {
            OpKind::Read => "reading from",
            OpKind::Write => "writing to",
        };
        write!(f, "I2C error {verb} {addr:?}: ")?;

        match kind {
            ErrorKind::I2c(kind) => kind.fmt(f),
            ErrorKind::NoDriver => "no driver task running".fmt(f),
        }
    }
}

impl i2c::Error for I2cError {
    fn kind(&self) -> i2c::ErrorKind {
        match self.kind {
            ErrorKind::I2c(kind) => kind,
            ErrorKind::NoDriver => i2c::ErrorKind::Other,
        }
    }
}
