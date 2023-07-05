use crate::{
    comms::{
        kchannel::{KChannel, KConsumer, KProducer},
        oneshot::{self, Reusable},
    },
    // buf::{ArrayBuf, OwnedReadBuf},
    mnemos_alloc::containers::FixedVec,
    registry::{known_uuids, Envelope, KernelHandle, OneshotRequestError, RegisteredDriver},
    Kernel,
};
use core::{convert::Infallible, fmt, time::Duration};
use embedded_hal_async::i2c;
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

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

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Addr {
    SevenBit(u8),
    TenBit(u16),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StartTransaction {
    pub addr: Addr,
    capacity: usize,
}

pub struct Transaction {
    addr: Addr,
    tx: KProducer<Op>,
    write_rx: Reusable<Result<WriteOp, i2c::ErrorKind>>,
    read_rx: Reusable<Result<ReadOp, i2c::ErrorKind>>,
}

#[derive(Debug)]
pub struct I2cError {
    addr: Addr,
    kind: ErrorKind,
    read: bool,
}

pub enum Op {
    Read(ReadOp, oneshot::Sender<Result<ReadOp, i2c::ErrorKind>>),
    Write(WriteOp, oneshot::Sender<Result<WriteOp, i2c::ErrorKind>>),
}

pub struct ReadOp {
    pub buf: FixedVec<u8>,
    pub len: usize,
}

pub struct WriteOp {
    pub buf: FixedVec<u8>,
    pub len: usize,
}

#[derive(Debug)]
enum ErrorKind {
    I2c(i2c::ErrorKind),
    Req(OneshotRequestError),
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

    pub async fn transaction(&mut self, addr: Addr) -> Transaction {
        let resp = self
            .handle
            .request_oneshot(StartTransaction { addr, capacity: 2 }, &self.reply)
            .await
            .unwrap();
        resp.body.expect("transaction should be created")
    }
}

impl Transaction {
    pub async fn new(
        StartTransaction { addr, capacity }: StartTransaction,
    ) -> (Self, KConsumer<Op>) {
        let (tx, rx) = KChannel::new_async(capacity).await.split();
        let read_rx = Reusable::new_async().await;
        let write_rx = Reusable::new_async().await;
        let txn = Transaction {
            addr,
            read_rx,
            write_rx,
            tx,
        };
        (txn, rx)
    }

    pub async fn read(&mut self, buf: FixedVec<u8>, len: usize) -> Result<FixedVec<u8>, I2cError> {
        let tx = self
            .read_rx
            .sender()
            .await
            .expect("read sender should not be in use");
        let op = ReadOp { buf, len };
        self.tx
            .enqueue_async(Op::Read(op, tx))
            .await
            .map_err(I2cError::mk_no_driver(self.addr, true))?;
        self.read_rx
            .receive()
            .await
            .map_err(I2cError::mk_no_driver(self.addr, true))?
            .map(|ReadOp { buf, .. }| buf)
            .map_err(I2cError::mk(self.addr, true))
    }

    pub async fn write(&mut self, buf: FixedVec<u8>, len: usize) -> Result<FixedVec<u8>, I2cError> {
        let tx = self
            .write_rx
            .sender()
            .await
            .expect("write sender should not be in use");
        let op = WriteOp { buf, len };
        self.tx
            .enqueue_async(Op::Write(op, tx))
            .await
            .map_err(I2cError::mk_no_driver(self.addr, false))?;
        self.write_rx
            .receive()
            .await
            .map_err(I2cError::mk_no_driver(self.addr, false))?
            .map(|WriteOp { buf, .. }| buf)
            .map_err(I2cError::mk(self.addr, false))
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
    pub fn is_read(&self) -> bool {
        self.read
    }

    pub fn new(addr: Addr, kind: i2c::ErrorKind, read: bool) -> Self {
        Self {
            addr,
            kind: ErrorKind::I2c(kind),
            read,
        }
    }

    fn mk(addr: Addr, read: bool) -> impl Fn(i2c::ErrorKind) -> Self {
        move |kind| Self {
            addr,
            kind: ErrorKind::I2c(kind),
            read,
        }
    }

    fn mk_client(addr: Addr, read: bool) -> impl Fn(OneshotRequestError) -> Self {
        move |e| Self {
            addr,
            kind: ErrorKind::Req(e),
            read,
        }
    }

    fn mk_no_driver<E>(addr: Addr, read: bool) -> impl Fn(E) -> Self {
        move |_| Self {
            addr,
            read,
            kind: ErrorKind::NoDriver,
        }
    }
}

impl fmt::Display for I2cError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { addr, kind, read } = self;
        let verb = match read {
            true => "reading from",
            false => "writing to",
        };
        write!(f, "I2C error {verb} {addr:?}: ")?;

        match kind {
            ErrorKind::I2c(kind) => kind.fmt(f),
            ErrorKind::Req(kind) => write!(f, "client error: {kind:?}"),
            ErrorKind::NoDriver => "no driver task running".fmt(f),
        }
    }
}

impl i2c::Error for I2cError {
    fn kind(&self) -> i2c::ErrorKind {
        match self.kind {
            ErrorKind::I2c(kind) => kind,
            ErrorKind::Req(_) => i2c::ErrorKind::Other,
            ErrorKind::NoDriver => i2c::ErrorKind::Other,
        }
    }
}
