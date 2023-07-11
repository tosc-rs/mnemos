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
///
/// This type represents a transaction consisting of a series of read and write
/// operations to a target device on an I<sup>2</sup>C bus with a given
/// [`Addr`]. This type is part of a lower-level interface for I<sup>2</sup>C
/// bus transactions, and is returned by the [`I2cClient::start_transaction`]
/// method.
///
/// Once a [`Transaction`] has been created by [`I2cClient::start_transaction`],
/// data can be written to the target device using the [`Transaction::write`]
/// method, and read from the target device using the [`Transaction::read`]
/// method. Any number of read and write operations may be performed within a
/// `Transaction` until an operation with `end: true` is performed. This
/// completes the transaction.
///
/// While a [`Transaction`] is in progress, the I<sup>2</sup>C bus is "locked"
/// by the client that is performing that transaction. Other clients calling
/// [`I2cClient::start_transaction`] (or using the
/// [`embedded_hal_async::i2c::I2c`] interface) will wait until the current
/// transaction has completed before beginning their own transactions.
#[must_use = "if a transaction has been started, it should be used to perform bus operations"]
pub struct Transaction {
    addr: Addr,
    tx: KProducer<Transfer>,
    rsp_rx: Reusable<Result<FixedVec<u8>, i2c::ErrorKind>>,
}

/// Errors returned by the [`I2cService`]
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

    /// An I<sup>2</sup>C bus transfer within a [`Transaction`].
    ///
    /// This message is sent to the [`I2cService`] by a [`Transaction`] in order
    /// to perform an individual bus read or write as part of that transaction.
    pub struct Transfer {
        /// A buffer to read bytes into (if `dir` is [`OpKind::Read`]), or write
        /// bytes from (if `dir` is [`OpKind::Write`]).
        ///
        /// If performing a write, this buffer is guaranteed to contain at least
        /// [`len`] bytes. The driver is expected to write the contents of this
        /// buffer to the I<sup>2</sup>C bus starting at `buf[0]` and ending at
        /// `buf[len - 1]`.
        ///
        /// If performing a read, this buffer is guaranteed to have at least
        /// [`len`] [*capacity*] remaining. The driver is expected to read by
        /// appending bytes to the end of this buffer until `len` bytes have
        /// been appended.
        ///
        /// [*capacity*]: mnemos_alloc::containers::FixedVec::capacity
        pub buf: FixedVec<u8>,
        /// The number of bytes to read or write in this I<sup>2</sup>C bus
        /// transfer.
        pub len: usize,
        /// If `true`, this transfer is the last transfer in the transaction.
        ///
        /// If `end` is `true`, the driver is expected to send a `STOP` condition
        /// when the transfer has completed. Otherwise, the driver should send a
        /// repeated `START`, as additional transfers will be performed.
        ///
        /// Once the driver has completed a transfer with `end == true`, it is
        /// permitted to return errors for any subsequent transfers in the
        /// current transaction.
        pub end: bool,
        /// Whether this is a read ([`OpKind::Read`]) or write
        /// ([`OpKind::Write`]) transfer.
        pub dir: OpKind,
        /// Sender for responses once the transfer has completed.
        ///
        /// Once the driver has completed the transfer, it is required to send
        /// back the `buf` received in this `Transfer` message if the transfer
        /// completed successfully.
        ///
        /// If the transfer was a read, then `buf` should contain the bytes read
        /// from the I<sup>2</sup>C bus. If the transfer was a write, buf may
        /// contain any data, or be empty.
        ///
        /// If the transfer could not be completed successfully, then the driver
        /// must send an [`i2c::ErrorKind`] indicating the cause of the failure,
        /// instead.
        pub rsp: oneshot::Sender<Result<FixedVec<u8>, i2c::ErrorKind>>,
    }

    /// Whether an I<sup>2</sup>C bus operation is a read or a write.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum OpKind {
        /// The operation is a read.
        Read,
        /// The operation is a write.
        Write,
    }
}

#[derive(Debug)]
enum ErrorKind {
    I2c(i2c::ErrorKind),
    NoDriver,
    BufTooSmall { len: usize, buf: usize },
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// A client for the [`I2cService`].
///
/// This type is used to perform I<sup>2</sup>C bus operations. It is obtained
/// using [`I2cClient::from_registry`] (or
/// [`I2cClient::from_registry_no_retry`]).
///
/// Once an `I2cClient` has been acquired, it may be used to perform
/// I<sup>2</sup>C operations, either using [its implementation of the
/// `embedded_hal_async` `I2c` trait][impl-i2c], or using the lower-level
/// [`Transaction`] interface returned by [`I2cClient::start_transaction`]. See
/// the documentation for [`embedded_hal_async::i2c::I2c`] for details on that
/// interface, or the [`Transaction`] type for details on using the lower-level
/// transaction interface.
///
/// An `I2cClient` does *not* represent a "lock" on the I<sup>2</sup>C bus.
/// Multiple `I2cClients` can coexist without preventing each other from
/// performing bus operations. Instead, the bus is locked only while performing
/// a [`Transaction`], or while using the [`I2c::transaction`] method on the
/// [`embedded_hal_async::i2c::I2c` implementation][impl-i2c].
///
/// [impl-i2c]: I2cClient#impl-I2c<u8>-for-I2cClient
#[must_use = "an `I2cClient` does nothing if it is not used to perform bus transactions"]
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

    /// Starts an I<sup>2</sup>C [`Transaction`] with the device at the provided
    /// `addr`.
    ///
    /// This method begins a bus transaction with the target device. While the
    /// returned [`Transaction`] type is held, other `I2cClients` cannot perform
    /// bus operations; the bus is released when the [`Transaction`] is dropped.
    ///
    /// After starting a [`Transaction`], the [`Transaction::read`] and
    /// [`Transaction::write`] methods are used to write to and read from the
    /// target I<sup>2</sup>C device. See the [`Transaction`] type's
    /// documentation for more information on how to use it.
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
    /// Constructs a new `Transaction` from the provided [`StartTransaction`]
    /// message, returning the `Transaction` and a [`KConsumer`] for receiving
    /// [`Transfer`]s within that `Transaction`.
    ///
    /// This is intended to be used by server implementations of the
    /// [`I2cService`] when handling [`StartTransaction`] messages.
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
    /// `read` and [`write`](Self::write) operations.
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
    /// # Errors
    ///
    /// - If an error occurs performing the I<sup>2</sup>C bus transaction.
    /// - If `len` is greater than [`buf.capacity()`] - [`buf.len()`].
    /// - If there is no [`I2cService`] running.
    ///
    /// # Cancelation Safety
    ///
    /// If this future is dropped, the underlying I<sup>2</sup>C bus read
    /// operation may still be performed.
    ///
    /// [`FixedVec`]: mnemos_alloc::containers::FixedVec
    /// [`buf.len()`]: mnemos_alloc::containers::FixedVec::len
    /// [`buf.capacity()`]: mnemos_alloc::containers::FixedVec::capacity
    pub async fn read(
        &mut self,
        buf: FixedVec<u8>,
        len: usize,
        end: bool,
    ) -> Result<FixedVec<u8>, I2cError> {
        // if there's already data in the buffer, the available capacity is the
        // total capacity minus the length of the existing data.
        let cap = buf.capacity() - buf.len();
        if cap < len {
            return Err(self.buf_too_small(OpKind::Read, len, cap));
        }
        self.xfer(buf, len, end, OpKind::Read).await
    }

    /// Write `len` bytes `buf` to the I<sup>2</sup>C.
    ///
    /// Note that, rather than always writing all the bytes in the buffer, this
    /// method takes a `len` argument which specifies the number of bytes to
    /// write. This is intended to allow callers to reuse the same [`FixedVec`]
    /// for multiple [`read`](Self::read) and `write` operations.
    ///
    /// # Arguments
    ///
    /// - `buf`: a [`FixedVec`] buffer containing at least `len` bytes to write
    ///   to the I<sup>2</sup>C bus.
    /// - `len`: the number of bytes to write from `buf`. This must be less than
    ///   or equal to [`buf.len()`].
    /// - `end`: whether or not to end the transaction. If this is `true`, a
    ///   `STOP` condition will be sent on the bus once `len` bytes have been
    ///   written. If this is `false`, a repeated `START` condition will be sent on
    ///   the bus once `len` bytes have been written.
    ///
    /// # Errors
    ///
    /// - If an error occurs performing the I<sup>2</sup>C bus transaction.
    /// - If `len` is greater  than [`buf.len()`].
    /// - If there is no [`I2cService`] running.
    ///
    /// # Cancelation Safety
    ///
    /// If this future is dropped, the underlying I<sup>2</sup>C bus write
    /// operation may still be performed.
    ///
    /// [`FixedVec`]: mnemos_alloc::containers::FixedVec
    /// [`buf.len()`]: mnemos_alloc::containers::FixedVec::len
    pub async fn write(
        &mut self,
        buf: FixedVec<u8>,
        len: usize,
        end: bool,
    ) -> Result<FixedVec<u8>, I2cError> {
        if buf.len() < len {
            return Err(self.buf_too_small(OpKind::Write, len, buf.len()));
        }
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
            .map_err(self.mk_no_driver_err(dir))?;
        self.rsp_rx
            .receive()
            .await
            .map_err(self.mk_no_driver_err(dir))?
            .map_err(self.mk_err(dir))
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
    /// Constructs a new `I2cError` from an
    /// [`embedded_hal_async::i2c::ErrorKind`].
    ///
    /// This method is intended to be used by implementations of the
    /// [`I2cService`] when they encounter an error performing an I<sup>2</sup>C
    /// operation.
    #[must_use]
    pub fn new(addr: Addr, kind: i2c::ErrorKind, dir: OpKind) -> Self {
        Self {
            addr,
            kind: ErrorKind::I2c(kind),
            dir,
        }
    }

    /// Returns whether this error occurred while performing an I<sup>2</sup>C
    /// [`read`](Transaction::read) or [`write`](Transaction::write) operation.
    #[inline]
    #[must_use]
    pub fn operation(&self) -> OpKind {
        self.dir
    }
}

impl Transaction {
    fn buf_too_small(&self, dir: OpKind, len: usize, buf: usize) -> I2cError {
        I2cError {
            addr: self.addr,
            kind: ErrorKind::BufTooSmall { len, buf },
            dir,
        }
    }

    fn mk_err(&self, dir: OpKind) -> impl Fn(i2c::ErrorKind) -> I2cError {
        let addr = self.addr;
        move |kind| I2cError {
            addr,
            kind: ErrorKind::I2c(kind),
            dir,
        }
    }

    fn mk_no_driver_err<E>(&self, dir: OpKind) -> impl Fn(E) -> I2cError {
        let addr = self.addr;
        move |_| I2cError {
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

        match (kind, dir) {
            (ErrorKind::I2c(kind), _) => kind.fmt(f),
            (ErrorKind::NoDriver, _) => "no driver task running".fmt(f),
            (ErrorKind::BufTooSmall { len, buf }, OpKind::Read) => write!(
                f,
                "remaining buffer capacity ({buf}B) too small for desired read length ({len}B)"
            ),
            (ErrorKind::BufTooSmall { len, buf }, OpKind::Write) => write!(
                f,
                "input buffer does not contain enough bytes (found {buf}B, needed to write {len}B)",
            ),
        }
    }
}

impl i2c::Error for I2cError {
    fn kind(&self) -> i2c::ErrorKind {
        match self.kind {
            ErrorKind::I2c(kind) => kind,
            _ => i2c::ErrorKind::Other,
        }
    }
}
