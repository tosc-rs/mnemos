use mnemos_alloc::containers::HeapArray;
use uuid::Uuid;

use crate::{
    comms::oneshot::Reusable,
    registry::{known_uuids, Envelope, KernelHandle, RegisteredDriver},
};
use core::fmt;
use embedded_hal_async::i2c;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct I2cService;

impl RegisteredDriver for I2cService {
    type Request = Request;
    type Response = Response;
    type Error = I2cError;

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

pub struct Request {
    pub addr: Addr,
    pub xfer: Transfer,
}

pub struct Response {
    xfer: Transfer,
}

#[derive(Debug)]
pub struct I2cError {
    addr: Addr,
    kind: ErrorKind,
    is_read: bool,
}

pub enum Transfer {
    Single(Op),
    ReadWrite {
        read: HeapArray<u8>,
        read_len: usize,
        write: HeapArray<u8>,
        write_len: usize,
    },
    Transaction(HeapArray<Op>),
}

pub enum Op {
    Read { buf: HeapArray<u8>, len: usize },
    Write { buf: HeapArray<u8>, len: usize },
}

#[derive(Debug)]
enum ErrorKind {
    I2c(i2c::ErrorKind),
    Req(OneshotRequestError),
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////
pub struct I2cClient {
    handle: KernelHandle<I2cService>,
    reply: Reusable<Envelope<Result<Response, I2cError>>>,
}

impl I2cClient {
    /// Read `len` bytes from the I<sup>2</sup>C device at `addr` into `buf`,
    /// returning `buf` on success.
    // TODO(eliza): return the buffer if the read fails, too!
    pub async fn read_into(
        &mut self,
        addr: Addr,
        len: usize,
        buf: HeapArray<u8>,
    ) -> Result<HeapArray<u8>, I2cError> {
        assert!(
            buf.len() >= len,
            "insufficent space in buffer for requested read from {addr:?}! \
            buf.len() = {}, read len = {len}",
            buf.len(),
        );

        let xfer = Transfer::Single(Op::Read { buf, len });
        let rsp = self
            .handle
            .request_oneshot(Request { addr, xfer }, &self.reply)
            .await
            .map_err(I2cError::mk_client(addr, true))?;
        let Response { xfer } = rsp.body?;
        match xfer {
            Transfer::Single(Op::Read { buf, len: read_len }) => {
                assert_eq!(
                    len, read_len,
                    "I2C service responded with a wrong sized read!"
                );
                Ok(buf)
            }
            _ => unreachable!(
                "i2c service replied to a single-read transfer with a \
                totally unrelated response transfer type. this is a bug!"
            ),
        }
    }

    /// Write `len` bytes from `buf` to the I<sup>2</sup>C device at `addr`,
    /// returning `buf` on success.
    // TODO(eliza): return the buffer if the write fails, too!
    pub async fn write_from(
        &mut self,
        addr: Addr,
        len: usize,
        buf: HeapArray<u8>,
    ) -> Result<HeapArray<u8>, I2cError> {
        assert!(
            buf.len() >= len,
            "buffer contains fewer bytes than the requested write to {addr:?}! \
            buf.len() = {}, read len = {len}",
            buf.len(),
        );

        let xfer = Transfer::Single(Op::Write { buf, len });
        let rsp = self
            .handle
            .request_oneshot(Request { addr, xfer }, &self.reply)
            .await
            .map_err(I2cError::mk_client(addr, false))?;
        let Response { xfer } = rsp.body?;
        match xfer {
            Transfer::Single(Op::Write { buf, len: read_len }) => {
                assert_eq!(
                    len, read_len,
                    "I2C service responded with a wrong sized write!"
                );
                Ok(buf)
            }
            _ => unreachable!(
                "i2c service replied to a single-write transfer with a \
                totally unrelated response transfer type. this is a bug!"
            ),
        }
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

// === impl I2cError ===

impl I2cError {
    pub fn is_read(&self) -> bool {
        self.is_read
    }

    pub fn new(addr: Addr, kind: i2c::ErrorKind, is_read: bool) -> Self {
        Self {
            addr,
            kind: ErrorKind::I2c(kind),
            is_read,
        }
    }

    fn mk_client(addr: Addr, is_read: bool) -> impl Fn(OneshotRequestError) -> Self {
        move |e| Self {
            addr,
            kind: ErrorKind::Req(e),
            is_read,
        }
    }
}

impl fmt::Display for I2cError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            addr,
            kind,
            is_read,
        } = self;
        let verb = match is_read {
            true => "reading from",
            false => "writing to",
        };
        match kind {
            ErrorKind::I2c(kind) => write!(f, "I2C driver error {verb} {addr:?}: {kind}"),
            ErrorKind::Req(kind) => write!(f, "I2C client error {verb} {addr:?}: {kind:?}"),
        }
    }
}

impl i2c::Error for I2cError {
    fn kind(&self) -> i2c::ErrorKind {
        match self.kind {
            ErrorKind::I2c(kind) => kind,
            ErrorKind::Req(_) => i2c::ErrorKind::Other,
        }
    }
}
