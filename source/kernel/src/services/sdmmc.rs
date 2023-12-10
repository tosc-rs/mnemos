//! SD/MMC Driver Service
//!
//! Driver for SD memory cards, SDIO cards and (e)MMC.
//! This kernel driver will implement the actual protocol
//! (which commands to send and how to interpret the response),
//! while the platform driver will implement the device specific part
//! (how to send and receive the data).
#![warn(missing_docs)]
use crate::{
    comms::oneshot::Reusable,
    mnemos_alloc::containers::FixedVec,
    registry::{self, known_uuids, Envelope, KernelHandle, RegisteredDriver},
    Kernel,
};
use maitake::time::{self, Duration};
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
    /// Hardware specific configuration that should be applied
    pub options: HardwareOptions,
    /// The type of command
    pub kind: CommandKind,
    /// The expected type of the response
    pub rsp_type: ResponseType,
    /// Whether the response CRC needs to be checked
    pub rsp_crc: bool,
    /// Optional buffer for incoming or outgoing data
    pub buffer: Option<FixedVec<u8>>,
}

/// The number of lines that are used for data transfer
#[derive(Debug, Eq, PartialEq)]
pub enum BusWidth {
    /// 1-bit bus width, default after power-up or idle
    Single,
    /// 4-bit bus width, also called wide bus operation mode for SD cards
    Quad,
    /// 8-bit bus width, only available for MMC
    Octo,
}

/// Hardware configuration that should be applied as part of the command
#[derive(Debug, Eq, PartialEq)]
pub enum HardwareOptions {
    /// No change in configuration
    None,
    /// Switch the bus width
    SetBusWidth(BusWidth),
}

/// The different types of commands that can be sent to the card
#[derive(Debug, Eq, PartialEq)]
pub enum CommandKind {
    /// Command without data transfer
    Control,
    /// Command for reading data, contains the number of bytes to read
    Read(usize),
    /// Command for writing data, contains the number of bytes to write
    Write(usize),
}

/// The different types of responses that can be sent by the card
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

/// Response returned by the card, can be short or long, depending on command
#[must_use]
pub enum Response {
    /// The 32-bit value from the 48-bit response.
    /// Potentially also includes the data buffer if this was a read command
    Short {
        /// The response on the command line
        value: u32,
        /// The received data, in case of a read command
        data: Option<FixedVec<u8>>,
    },
    /// The 128-bit value from the 136-bit response
    Long(u128),
}

/// Errors returned by the [`SdmmcService`]
#[derive(Debug, Eq, PartialEq)]
pub struct Error {
    kind: ErrorKind,
    message: Option<&'static str>,
}

/// The different types of errors that can occur
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The service is currently busy and cannot handle the request
    Busy,
    /// Invalid or unexpected response was received
    Response,
    /// Invalid or unexpected data was received
    Data,
    /// The provided buffer does not meet the requirements
    Buffer,
    /// A timeout occurred
    Timeout,
    /// A different error has occurred
    Other,
}

impl Default for Command {
    fn default() -> Self {
        Command {
            index: 0,
            argument: 0,
            options: HardwareOptions::None,
            kind: CommandKind::Control,
            rsp_type: ResponseType::None,
            rsp_crc: false,
            buffer: None,
        }
    }
}

impl Error {
    /// Create an error from a type and message
    pub fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self {
            kind,
            message: Some(message),
        }
    }
}

impl From<ErrorKind> for Error {
    fn from(value: ErrorKind) -> Self {
        Self {
            kind: value,
            message: None,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SD/MMC error: ")?;
        match self.kind {
            ErrorKind::Busy => f.write_str("busy "),
            ErrorKind::Response => f.write_str("response "),
            ErrorKind::Data => f.write_str("data "),
            ErrorKind::Buffer => f.write_str("buffer "),
            ErrorKind::Timeout => f.write_str("timeout "),
            ErrorKind::Other => f.write_str("other "),
        }?;

        self.message.map_or_else(|| Ok(()), |msg| msg.fmt(f))
    }
}

impl core::fmt::Debug for Response {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Response::Short { value, data: _ } => write!(f, "{value:#x}"),
            Response::Long(value) => write!(f, "{value:#x}"),
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

/// The different types of cards
#[derive(Debug, PartialEq)]
pub enum CardType {
    /// Standard capacity v1
    SD1,
    /// Standard capacity v2 (or later)
    SD2,
    /// High capacity
    SDHC,
}

/// Card status in R1 response format
#[derive(Debug, PartialEq)]
pub struct CardStatus(u32);

/// Card identification register in R2 response format
///
/// | Field                 | Bits        |
/// |:----------------------|------------:|
/// | Manufacturer ID       | `[127:120]` |
/// | OEM/Application ID    | `[119:104]` |
/// | Product name          | `[103:64]`  |
/// | Product revision      | `[63:56]`   |
/// | Product serial number | `[55:24]`   |
/// | Reserved              | `[23:20]`   |
/// | Manufacturing date    | `[19:8]`    |
/// | CRC7 checksum         | `[7:1]`     |
/// | Not used, always 1    | `[0:0]`     |
pub struct CardIdentification(u128);

/// Published RCA in R6 response format
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct RelativeCardAddress(u32);

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

    async fn cmd(&mut self, cmd: Command) -> Result<Response, Error> {
        let index = cmd.index;
        let result = self
            .handle
            .request_oneshot(cmd, &self.reply)
            .await
            .map_err(|error| {
                tracing::warn!(
                    ?error,
                    "failed to send CMD{index} request to SD/MMC service"
                );
                Error::from(ErrorKind::Other) // TODO
            })
            .and_then(|resp| resp.body);
        tracing::trace!("CMD{index} response: {result:?}");
        result
    }

    /// Reset the card
    pub async fn reset(&mut self) -> Result<(), Error> {
        self.cmd(Command::default()).await.map(|_| ())
    }

    /// Initialize the card
    pub async fn initialize(&mut self) -> Result<CardType, Error> {
        /// Request switch to 1.8V
        #[allow(dead_code)]
        const OCR_S18R: u32 = 0x1000000;
        /// Host supports high capacity
        const OCR_HCS: u32 = 0x40000000;
        /// Card has finished power up routine if bit is high
        const OCR_NBUSY: u32 = 0x80000000;
        /// Valid bits for voltage setting
        const OCR_VOLTAGE_MASK: u32 = 0x007FFF80;
        // Set 'voltage supplied' to 2.7-3.6V and 'check pattern' to 0xAA
        const CMD8_ARG: u32 = 0x1AA;

        let mut card_type = CardType::SD1;

        match self
            .cmd(Command {
                index: 8,
                argument: CMD8_ARG,
                options: HardwareOptions::None,
                kind: CommandKind::Control,
                rsp_type: ResponseType::Short,
                rsp_crc: true,
                buffer: None,
            })
            .await?
        {
            Response::Short { value, .. } => {
                if value == CMD8_ARG {
                    card_type = CardType::SD2;
                }
            }
            Response::Long(_) => return Err(Error::from(ErrorKind::Response)),
        }

        // TODO: limit the number of attempts
        let ocr = loop {
            // Go to *APP* mode before sending application command
            let _ = self
                .cmd(Command {
                    index: 55,
                    argument: 0,
                    options: HardwareOptions::None,
                    kind: CommandKind::Control,
                    rsp_type: ResponseType::Short,
                    rsp_crc: true,
                    buffer: None,
                })
                .await?;

            let mut op_cond_arg = OCR_VOLTAGE_MASK & 0x00ff8000;
            if card_type != CardType::SD1 {
                op_cond_arg |= OCR_HCS;
            }
            match self
                .cmd(Command {
                    index: 41,
                    argument: op_cond_arg,
                    options: HardwareOptions::None,
                    kind: CommandKind::Control,
                    rsp_type: ResponseType::Short,
                    rsp_crc: false,
                    buffer: None,
                })
                .await?
            {
                Response::Short { value, .. } => {
                    if value & OCR_NBUSY == OCR_NBUSY {
                        // Card has finished power up, data is valid
                        break value;
                    }
                }
                Response::Long(_) => return Err(Error::from(ErrorKind::Response)),
            }

            time::sleep(Duration::from_millis(1)).await;
        };

        if (ocr & OCR_HCS) == OCR_HCS {
            card_type = CardType::SDHC;
        }

        Ok(card_type)
    }

    /// Get the card identification register
    pub async fn get_cid(&mut self) -> Result<CardIdentification, Error> {
        match self
            .cmd(Command {
                index: 2,
                argument: 0,
                options: HardwareOptions::None,
                kind: CommandKind::Control,
                rsp_type: ResponseType::Long,
                rsp_crc: true,
                buffer: None,
            })
            .await?
        {
            Response::Short { .. } => Err(Error::from(ErrorKind::Response)),
            Response::Long(value) => Ok(CardIdentification(value)),
        }
    }

    /// Get the relative card address
    pub async fn get_rca(&mut self) -> Result<RelativeCardAddress, Error> {
        match self
            .cmd(Command {
                index: 3,
                argument: 0,
                options: HardwareOptions::None,
                kind: CommandKind::Control,
                rsp_type: ResponseType::Short,
                rsp_crc: true,
                buffer: None,
            })
            .await?
        {
            Response::Short { value, .. } => Ok(RelativeCardAddress(value)),
            Response::Long(_) => Err(Error::from(ErrorKind::Response)),
        }
    }

    /// Toggle the card between stand-by and transfer state
    pub async fn select(&mut self, rca: RelativeCardAddress) -> Result<CardStatus, Error> {
        match self
            .cmd(Command {
                index: 7,
                argument: rca.0,
                options: HardwareOptions::None,
                kind: CommandKind::Control,
                rsp_type: ResponseType::Short,
                rsp_crc: true,
                buffer: None,
            })
            .await?
        {
            Response::Short { value, .. } => Ok(CardStatus(value)),
            Response::Long(_) => Err(Error::from(ErrorKind::Response)),
        }
    }

    /// Use 4 data lanes
    pub async fn set_wide_bus(&mut self, rca: RelativeCardAddress) -> Result<CardStatus, Error> {
        // Go to *APP* mode before sending application command
        let _ = self
            .cmd(Command {
                index: 55,
                argument: rca.0,
                options: HardwareOptions::None,
                kind: CommandKind::Control,
                rsp_type: ResponseType::Short,
                rsp_crc: true,
                buffer: None,
            })
            .await?;

        match self
            .cmd(Command {
                index: 6,
                argument: 0b10, // instruct card to use 4-bits bus
                options: HardwareOptions::SetBusWidth(BusWidth::Quad),
                kind: CommandKind::Control,
                rsp_type: ResponseType::Short,
                rsp_crc: true,
                buffer: None,
            })
            .await?
        {
            Response::Short { value, .. } => Ok(CardStatus(value)),
            Response::Long(_) => Err(Error::from(ErrorKind::Response)),
        }
    }

    /// Read the desired number of data blocks into the provided buffer, starting at the given sector.
    pub async fn read(
        &mut self,
        sector: u32,
        blocks: usize,
        buf: FixedVec<u8>,
    ) -> Result<FixedVec<u8>, Error> {
        const BLOCK_SIZE: usize = 512;

        let bytes = BLOCK_SIZE * blocks;
        // The provider buffer should have space for the requested amount of data
        if buf.capacity() < bytes {
            return Err(Error::from(ErrorKind::Buffer));
        }

        match self
            .cmd(Command {
                index: 18,
                argument: sector,
                options: HardwareOptions::None,
                kind: CommandKind::Read(bytes),
                rsp_type: ResponseType::Short,
                rsp_crc: true,
                buffer: Some(buf),
            })
            .await?
        {
            Response::Short {
                data: Some(res), ..
            } => Ok(res),
            _ => Err(Error::from(ErrorKind::Response)),
        }
    }
}

/// A client for SDIO cards using the [`SdmmcService`].
#[allow(dead_code)]
pub struct SdioClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

/// A client for MMC cards using the [`SdmmcService`].
#[allow(dead_code)]
pub struct MmcClient {
    handle: KernelHandle<SdmmcService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}
