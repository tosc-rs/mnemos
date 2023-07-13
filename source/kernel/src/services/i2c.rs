//! I²C Driver Service
//!
//! This module contains a service definition for drivers for the I²C
//! bus.
//!
//! ## About I²C
//!
//! I²C, according to the [RP2040 datasheet], is "an ubiquitous
//! serial bus first described in the Dead Sea Scrolls, and later used by
//! Philips Semiconductor". It's a two-wire, multi-drop bus, allowing multiple
//! devices to be connected to a single clock and data line.
//!
//! Unlike SPI, this is a "real protocol", and not just a sort of shared
//! hallucination about the meanings of certain wires. That means it has
//! _rules_. Some of these rules are relevant to users of this module. In
//! particular:
//!
//! * I²C has a first-class notion of controller and target devices.
//!   The bus has a single controller (formerly referred to as the "master"),
//!   which initiates all bus operations. All other devices are targets
//!   (formerly, insensitively referred to as "slaves"), which may only respond
//!   to operations that target their address. The interfaces in this module
//!   assume that the MnemOS kernel is running on the device acting as the bus
//!   controller.
//! * In order to communicate with a target device, the controller must first
//!   send a `START` condition on the bus. When the controller has finished
//!   communicating with that device, it will send a `STOP` condition. If the
//!   controller completes a read or write operation and wishes to perform
//!   additional read or write operations with the same device, it may instead
//!   send a repeated `START` condition. Therefore, whether a bus operation
//!   should end with a `STOP` or with a `START` depends on whether the user
//!   intends to perform additional operations with that device as part of the
//!   same transaction. The [`Transaction`] interface in this module allows the
//!   user to indicate whether a read or write operation should end the bus
//!   transaction. The [`embedded_hal_async::i2c::I2c`] trait also has an
//!   [`I2c::transaction`] method, which may be used to perform multiple read
//!   and write operations within the same transaction.
//!
//! ## Usage
//!
//! Users of the I²C bus will primarily interact with this module
//! using the [`I2cClient`] type, which implements a client for the
//! [`I2cService`] service. This client type can be used to perform read and
//! write operations on the I²C bus. A new client can be acquired
//! using [`I2cClient::from_registry`].
//!
//! Once an [`I2cClient`] has been obtained, it can be used to perform
//! I²C operations. Two interfaces are available: an
//! [implementation][impl-i2c] of the [`embedded_hal_async::i2c::I2c`] trait,
//! and a lower-level interface using the [`I2cClient::start_transaction`]
//! method. In general, the [`embedded_hal_async::i2c::I2c`] trait is the
//! recommended interface.
//!
//! The lower-level interface allows reusing the same heap-allocated buffer for
//! multiple I²C bus transactions. It also provides the ability to
//! interleave other code between the write and read operations of an
//! I²C transaction without sending a STOP condition. If either of
//! these are necessary, the [`Transaction`] interface may be preferred over the
//! [`embedded_hal_async`] interface.
//!
//! ### On Buffer Reuse
//!
//! Because of mnemOS' message-passing design, the [`I2cService`] operates
//! with owned buffers, rather than borrowed buffers, so a [`FixedVec`]`<u8>` is
//! used as the buffer type for both read and write operations. This means
//! that we must allocate when performing I²C operations. To
//! reduce the amount of allocation necessary, all [`Transaction`] methods
//! return the buffer that was passed in, allowing the buffer to be reused
//! for multiple operations.
//!
//! To facilitate this, the [`Transaction::read`]  method also takes a
//! `len` parameter indicating the actual number of bytes to read into
//! the buffer, rather than always filling the  entire buffer with
//! bytes. This way, we can size the buffer to the largest buffer
//! required for a sequence of operations, but perform smaller reads and
//! writes using the same [`FixedVec`]`<u8>`, avoiding reallocations.
//! The implementation of [`embedded_hal_async::i2c::I2c::transaction`]
//! will allocate a single buffer large enough for the largest operation
//! in the transaction, and reuse that buffer for every operation within
//! the transaction.
//!
//! Note that the [`Transaction::write`] method does *not* need to take a `len`
//! parameter, and will always write all bytes currently in the buffer. The
//! `len` parameter is only needed for [`Transaction::read`], because reads are
//! limited by the buffer's *total capacity*, rather than the current length of
//! the initialized portion.
//!
//! [RP2040 datasheet]: https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf
//! [impl-i2c]: I2cClient#impl-I2c<u8>-for-I2cClient
//! [`I2c::transaction`]: embedded_hal_async::i2c::I2c::transaction
//! [`FixedVec`]: mnemos_alloc::containers::FixedVec
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

/// [Service](crate::services) definition for I²C bus drivers.
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

/// I²C bus address, in either 7-bit or 10-bit address format.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Addr {
    /// A 7-bit I²C address.
    SevenBit(u8),
    /// A 10-bit extended I²C address.
    TenBit(u16),
}

/// A transaction on the I²C bus.
///
/// This type represents a transaction consisting of a series of read and write
/// operations to a target device on an I²C bus with a given
/// [`Addr`]. This type is part of a lower-level interface for I²C
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
/// While a [`Transaction`] is in progress, the I²C bus is "locked"
/// by the client that is performing that transaction. Other clients calling
/// [`I2cClient::start_transaction`] (or using the
/// [`embedded_hal_async::i2c::I2c`] interface) will wait until the current
/// transaction has completed before beginning their own transactions.
#[must_use = "if a transaction has been started, it should be used to perform bus operations"]
pub struct Transaction {
    addr: Addr,
    tx: KProducer<Transfer>,
    rsp_rx: Reusable<Result<FixedVec<u8>, i2c::ErrorKind>>,
    ended: bool,
}

/// Errors returned by the [`I2cService`]
#[derive(Debug)]
pub struct I2cError {
    addr: Addr,
    kind: ErrorKind,
    dir: OpKind,
}

/// Messages used to communicate with an [`I2cService`] implementation.
///
/// The types in this module are primarily used by implementations of the
/// [`I2cService`], and are not relevant to users of the [`I2cClient`]
/// interface.
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

    /// An I²C bus transfer within a [`Transaction`].
    ///
    /// This message is sent to the [`I2cService`] by a [`Transaction`] in order
    /// to perform an individual bus read or write as part of that transaction.
    pub struct Transfer {
        /// A buffer to read bytes into (if [`dir`] is [`OpKind::Read`]), or write
        /// bytes from (if `dir` is [`OpKind::Write`]).
        ///
        /// If performing a write, this buffer is guaranteed to contain at least
        /// [`len`] bytes. The driver is expected to write the contents of this
        /// buffer to the I²C bus starting at `buf[0]` and ending at
        /// `buf[len - 1]`.
        ///
        /// If performing a read, this buffer is guaranteed to have at least
        /// [`len`] [*capacity*] remaining. The driver is expected to read by
        /// appending bytes to the end of this buffer until `len` bytes have
        /// been appended.
        ///
        /// [`dir`]: #structfield.dir
        /// [`len`]: #structfield.len
        /// [*capacity*]: mnemos_alloc::containers::FixedVec::capacity
        pub buf: FixedVec<u8>,
        /// The number of bytes to read or write from [`buf`] in this I²C bus
        /// transfer.
        ///
        /// [`buf`]: #structfield.buf
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
        /// back the [`buf`] received in this `Transfer` message if the transfer
        /// completed successfully.
        ///
        /// If the transfer was a read, then [`buf`] should contain the bytes read
        /// from the I²C bus. If the transfer was a write, buf may
        /// contain any data, or be empty.
        ///
        /// If the transfer could not be completed successfully, then the driver
        /// must send an [`i2c::ErrorKind`] indicating the cause of the failure,
        /// instead.
        ///
        /// [`buf`]: #structfield.buf
        pub rsp: oneshot::Sender<Result<FixedVec<u8>, i2c::ErrorKind>>,
    }

    /// Whether an I²C bus operation is a read or a write.
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
    ReadBufTooSmall { len: usize, cap: usize },
    AlreadyEnded,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// A client for the [`I2cService`].
///
/// This type is used to perform I²C bus operations. It is obtained
/// using [`I2cClient::from_registry`] (or
/// [`I2cClient::from_registry_no_retry`]).
///
/// Once an `I2cClient` has been acquired, it may be used to perform
/// I²C operations, either using [its implementation of the
/// `embedded_hal_async` `I2c` trait][impl-i2c], or using the lower-level
/// [`Transaction`] interface returned by [`I2cClient::start_transaction`]. See
/// the documentation for [`embedded_hal_async::i2c::I2c`] for details on that
/// interface, or the [`Transaction`] type for details on using the lower-level
/// transaction interface.
///
/// An `I2cClient` does *not* represent a "lock" on the I²C bus.
/// Multiple `I2cClients` can coexist without preventing each other from
/// performing bus operations. Instead, the bus is locked only while performing
/// a [`Transaction`], or while using the [`I2c::transaction`] method on the
/// [`embedded_hal_async::i2c::I2c` implementation][impl-i2c].
///
/// [impl-i2c]: I2cClient#impl-I2c<u8>-for-I2cClient
/// [`I2c::transaction`]: embedded_hal_async::i2c::I2c::transaction
#[must_use = "an `I2cClient` does nothing if it is not used to perform bus transactions"]
pub struct I2cClient {
    handle: KernelHandle<I2cService>,
    reply: Reusable<Envelope<Result<Transaction, Infallible>>>,
}

impl I2cClient {
    /// Obtain an `I2cClient`
    ///
    /// If the [`I2cService`] hasn't been registered yet, we will retry until it
    /// has been registered.
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

    /// Starts an I²C [`Transaction`] with the device at the provided
    /// `addr`.
    ///
    /// This method begins a bus transaction with the target device. While the
    /// returned [`Transaction`] type is held, other `I2cClients` cannot perform
    /// bus operations; the bus is released when the [`Transaction`] is dropped.
    ///
    /// After starting a [`Transaction`], the [`Transaction::read`] and
    /// [`Transaction::write`] methods are used to write to and read from the
    /// target I²C device. See the [`Transaction`] type's
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
            crate::tracing::trace!(n, n_ops, ?op, ?end);
            match op {
                i2c::Operation::Read(dest) => {
                    let len = dest.len();
                    let read = txn.read(buf, len, end).await?;
                    dest.copy_from_slice(read.as_slice());
                    buf = read;
                }
                i2c::Operation::Write(src) => {
                    buf.try_extend_from_slice(src)
                        .expect("we should have pre-allocated a large enough buffer!");
                    let write = txn.write(buf, end).await?;
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
        let txn = Transaction {
            addr,
            rsp_rx,
            tx,
            ended: false,
        };
        (txn, rx)
    }

    /// Read `len` bytes from the I²C bus into `buf`.
    ///
    /// Note that, rather than always filling the entire buffer, this method
    /// takes a `len` argument which specifies the number of bytes to read. This
    /// is intended to allow callers to reuse the same [`FixedVec`] for multiple
    /// `read` and [`write`](Self::write) operations.
    ///
    /// # Arguments
    ///
    /// - `buf`: a [`FixedVec`] buffer into which bytes read from the
    ///   I²C bus will be written
    /// - `len`: the number of bytes to read. This must be less than or equal to
    ///   `buf.len()`.
    /// - `end`: whether or not to end the transaction. If this is `true`, a
    ///   `STOP` condition will be sent on the bus once `len` bytes have been
    ///   read. If this is `false`, a repeated `START` condition will be sent on
    ///   the bus once `len` bytes have been read.
    ///
    /// # Errors
    ///
    /// - If an error occurs performing the I²C bus transaction.
    /// - If `len` is greater than [`buf.capacity()`] - [`buf.len()`].
    /// - If there is no [`I2cService`] running.
    ///
    /// # Cancelation Safety
    ///
    /// If this future is dropped, the underlying I²C bus read
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

    /// Write bytes from `buf` to the I²C.
    ///
    ///
    /// # Arguments
    ///
    /// - `buf`: a [`FixedVec`] buffer containing the bytes to write to the I²C bus.
    /// - `end`: whether or not to end the transaction. If this is `true`, a
    ///   `STOP` condition will be sent on the bus once the entire buffer has
    ///   been sent. If this is `false`, a repeated `START` condition will be
    ///   sent on the bus once the entire buffer has been written.
    ///
    /// # Errors
    ///
    /// - If an error occurs performing the I²C bus transaction.
    /// - If there is no [`I2cService`] running.
    ///
    /// # Cancelation Safety
    ///
    /// If this future is dropped, the underlying I²C bus write
    /// operation may still be performed.
    ///
    /// [`FixedVec`]: mnemos_alloc::containers::FixedVec
    /// [`buf.len()`]: mnemos_alloc::containers::FixedVec::len
    pub async fn write(&mut self, buf: FixedVec<u8>, end: bool) -> Result<FixedVec<u8>, I2cError> {
        let len = buf.len();
        self.xfer(buf, len, end, OpKind::Write).await
    }

    async fn xfer(
        &mut self,
        buf: FixedVec<u8>,
        len: usize,
        end: bool,
        dir: OpKind,
    ) -> Result<FixedVec<u8>, I2cError> {
        if self.ended {
            return Err(I2cError {
                dir,
                addr: self.addr,
                kind: ErrorKind::AlreadyEnded,
            });
        } else {
            self.ended = end;
        }

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
    /// [`I2cService`] when they encounter an error performing an I²C
    /// operation.
    #[must_use]
    pub fn new(addr: Addr, kind: i2c::ErrorKind, dir: OpKind) -> Self {
        Self {
            addr,
            kind: ErrorKind::I2c(kind),
            dir,
        }
    }

    /// Returns whether this error occurred while performing an I²C
    /// [`read`](Transaction::read) or [`write`](Transaction::write) operation.
    #[inline]
    #[must_use]
    pub fn operation(&self) -> OpKind {
        self.dir
    }

    /// Returns `true` if this `I2cError` represents an invalid use of the
    /// [`I2cClient`]/[`Transaction`] APIs.
    ///
    /// User errors include:
    ///
    /// - Attempting to read or write after performing a read or write operation
    ///   with `end: true`, ending the transaction.
    /// - Attempting to read or write with a buffer that is too small for the
    ///   desired read/write operation.
    #[must_use]
    #[inline]
    pub fn is_user_error(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::ReadBufTooSmall { .. } | ErrorKind::AlreadyEnded
        )
    }
}

impl Transaction {
    fn buf_too_small(&self, dir: OpKind, len: usize, buf: usize) -> I2cError {
        I2cError {
            addr: self.addr,
            kind: ErrorKind::ReadBufTooSmall { len, cap: buf },
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

        match kind {
            ErrorKind::I2c(kind) => kind.fmt(f),
            ErrorKind::NoDriver => "no driver task running".fmt(f),
            ErrorKind::AlreadyEnded => {
                "this transaction has already ended. start a new transaction.".fmt(f)
            }
            ErrorKind::ReadBufTooSmall { len, cap } => write!(
                f,
                "remaining buffer capacity ({cap}B) too small for desired read length ({len}B)"
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
