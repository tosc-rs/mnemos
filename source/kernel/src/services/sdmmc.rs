//! SD/MMC Driver Service
//!
//! TODO
// TODO: #![warn(missing_docs)]
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
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// [Service](crate::services) definition for SD/MMC protocol drivers.
pub struct SdmmcService;

impl RegisteredDriver for SdmmcService {
    type Request = StartTransaction;
    type Response = Transaction;
    type Error = core::convert::Infallible;

    const UUID: Uuid = known_uuids::kernel::SDMMC;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

/// TODO
#[must_use]
pub struct Transaction {
    tx: KProducer<Transfer>,
    rsp_rx: Reusable<Result<FixedVec<u8>, () /* TODO */>>,
    ended: bool,
}

pub mod messages {
    use super::*;

    pub struct StartTransaction {}
    pub struct Transfer {}
}
////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// A client for the [`SdmmcService`].
pub struct SdmmcClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Transaction, Infallible>>>,
}
