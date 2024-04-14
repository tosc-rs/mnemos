use crate::{comms::oneshot, mnemos_alloc::containers::FixedVec};

pub mod controller;

pub struct Transfer {
    pub req: TransferBufs,
    pub rsp: oneshot::Sender<TransferBufs>,
}

pub struct TransferBufs {
    /// Bytes to read.
    pub read: Option<Buf>,
    /// Bytes to write.
    pub write: Option<Buf>,
}

pub struct Buf {
    /// The buffer to read data into or write data from.
    pub buf: FixedVec<u8>,
    /// The number of bytes to read into or write from `buf`.
    pub len: usize,
}
