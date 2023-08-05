//! SD/MMC Driver Service
//!
//! Driver for SD memory cards, SDIO cards and (e)MMC cards.
//! This kernel driver will implement the actual protocol
//! (which commands to send and how to interpret the response),
//! while the platform driver will implement the device specific part
//! (how to send and receive the data).
#![warn(missing_docs)]
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
    type Request = Command;
    type Response = Response;
    type Error = core::convert::Infallible;

    const UUID: Uuid = known_uuids::kernel::SDMMC;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

/// Control or data command.
/// This is the same for SD and MMC (?), but the content should be different.
/// For data command, should also contain the buffers to read/write?
pub struct Command {
    /// The numeric value of the command
    command: u8,
    /// Argument value that should be sent on the control line
    argument: u32,
    /// The type of command
    cmd_type: CommandType,
    /// The expected length of the response
    rsp_size: ResponseLength,
    /// Will the card respond with a CRC that needs to be checked
    crc: bool,
    /// Incoming or outgoing data
    buffer: Option<FixedVec<u8>>,
}

/// TODO
pub enum CommandType {
    Control,
    Read,
    Write,
}

/// TODO
pub enum ResponseLength {
    /// 48-bits
    Short,
    /// 136-bits
    Long,
}

/// Response returned by the card, can be short or long, depending on command.
/// For read command, you can find the data in previous buffer?
#[must_use]
pub struct Response {
}

/// TODO
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// A client for SD memory cards using the [`SdmmcService`].
pub struct SdCardClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

/// A client for SDIO cards using the [`SdmmcService`].
pub struct SdioClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

/// A client for MMC cards using the [`SdmmcService`].
pub struct MmcClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}
