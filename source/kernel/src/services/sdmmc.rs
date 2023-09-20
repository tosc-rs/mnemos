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
    registry::{self, known_uuids, Envelope, KernelHandle, RegisteredDriver},
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
    type Error = Error;
    type Hello = ();
    type ConnectError = core::convert::Infallible;

    const UUID: Uuid = known_uuids::kernel::SDMMC;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

/// Parameters for building a command to be sent to the card.
///
/// The command format must follow the SD specification and is sent on the `CMD` line.
/// It is 48-bit in length, containing a 6-bit command index and 32-bit argument.
/// Besides that it includes 7-bit CRC and start, transmission and end bit.
///
/// The command structure is the same for SD memory, SDIO and MMC (?),
/// but the content can be very different.
/// Therefore the content of the commands is decided here in the kernel service,
/// while the platform driver has the low-level implementation
/// for how to physically send the necessary bits to the card.
pub struct Command {
    /// The numeric value of the command
    pub index: u8,
    /// The argument value for the command
    pub argument: u32,
    /// The type of command
    pub kind: CommandKind,
    /// The expected type of the response
    pub rsp_type: ResponseType,
    /// Whether the response CRC needs to be checked
    pub rsp_crc: bool,
    /// Optional buffer for incoming or outgoing data
    pub buffer: Option<FixedVec<u8>>,
}

/// TODO
#[derive(Debug, Eq, PartialEq)]
pub enum CommandKind {
    /// Command without data transfer
    Control,
    /// Command for reading data, contains the number of bytes to read
    Read(u32),
    /// Command for writing data, contains the number of bytes to write
    Write(u32),
}

/// TODO
#[derive(Debug, Eq, PartialEq)]
pub enum ResponseType {
    /// No Response
    None,
    /// Response with 48-bit length
    Short,
    /// Response with 48-bit length + check *busy* after response
    ShortWithBusySignal,
    /// Response with 136-bit length
    Long,
}

/// Response returned by the card, can be short or long, depending on command.
/// For read command, you can find the data in previous buffer?
#[must_use]
pub enum Response {
    /// The 32-bit value from the 48-bit response.
    /// Potentially also includes the data vector if this was a read command.
    Short {
        value: u32,
        data: Option<FixedVec<u8>>,
    },
    /// The 128-bit value from the 136-bit response.
    // TODO: make this `u128`?
    Long([u32; 4]),
}

/// TODO
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    /// TODO
    Busy,
    /// TODO
    Response,
    /// TODO
    Data,
    /// TODO
    Timeout,
    /// TODO
    Other,
}

impl Default for Command {
    fn default() -> Self {
        Command {
            index: 0,
            argument: 0,
            kind: CommandKind::Control,
            rsp_type: ResponseType::None,
            rsp_crc: false,
            buffer: None,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// A client for SD memory cards using the [`SdmmcService`].
pub struct SdCardClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

impl SdCardClient {
    /// Obtain an `SdCardClient`
    ///
    /// If the [`SdmmcService`] hasn't been registered yet, we will retry until it
    /// has been registered.
    pub async fn from_registry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SdmmcService>> {
        let handle = kernel.registry().connect::<SdmmcService>(()).await?;

        Ok(Self {
            handle,
            reply: Reusable::new_async().await,
        })
    }

    /// Obtain an `SdCardClient`
    ///
    /// Does NOT attempt to get an [`SdmmcService`] handle more than once.
    ///
    /// Prefer [`SdCardClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SdmmcService>> {
        let handle = kernel.registry().try_connect::<SdmmcService>(()).await?;

        Ok(Self {
            handle,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn reset(&mut self) {
        let response = self
            .handle
            .request_oneshot(Command::default(), &self.reply)
            .await
            .map_err(|error| {
                tracing::warn!(?error, "failed to send request to SD/MMC service");
                Error::Other // TODO
            })
            .and_then(|resp| resp.body);
    }

    pub async fn initialize(&mut self) {}
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
