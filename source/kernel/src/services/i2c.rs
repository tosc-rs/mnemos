use mnemos_alloc::containers::HeapFixedVec;
use uuid::Uuid;

use crate::{
    comms::oneshot::Reusable,
    registry::{known_uuids, Envelope, KernelHandle, RegisteredDriver},
};

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

pub struct Request {
    addr: u8,
    xfer: Transfer,
}

pub struct Response {
    xfer: Transfer,
}

enum Transfer {
    Single(Op),
    ReadWrite {
        read: HeapFixedVec<u8>,
        read_len: usize,
        write: HeapFixedVec<u8>,
        write_len: usize,
    },
    Transaction(HeapFixedVec<Op>),
}

struct Buf {}
enum Op {
    Read { buf: HeapFixedVec<u8>, len: usize },
    Write { buf: HeapFixedVec<u8>, len: usize },
}

/// Errors returned by `I2cService`.
pub enum I2cError {
    Nak,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////
pub struct I2cClient {
    kprod: KernelHandle<I2cService>,
    rosc: Reusable<Envelope<Result<Response, I2cError>>>,
}
