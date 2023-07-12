pub use bbq10kbd::{KeyRaw, KeyStatus, Version};
use kernel::{
    comms::kchannel::{KChannel, KConsumer, KProducer},
    registry::{RegisteredDriver, RegistrationError},
    services::i2c::{I2cClient, I2cError},
    trace::{self, instrument, Level},
    Kernel,
};
use uuid::{uuid, Uuid};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////
pub struct I2cPuppetService;

impl RegisteredDriver for I2cPuppetService {
    type Request = Request;
    type Response = Response;
    type Error = Error;

    const UUID: Uuid = uuid!("f5f26c40-6079-4233-8894-39887b878dec");
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////
pub enum Request {
    GetVersion,
    SetColor(RgbColor),
    ToggleLed(bool),
    GetLedStatus,
    SubscribeToKeys,
}

pub enum Response {
    GetVersion(Version),
    SetColor(RgbColor),
    ToggleLed(bool),
    GetLedStatus { color: RgbColor, on: bool },
    SubscribeToKeys(KConsumer<KeyRaw>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub enum Error {
    I2c(I2cError),
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct I2cPuppetClient {}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

/// Server implementation for the [`I2cPuppetService`].
pub struct I2cPuppetServer;

impl I2cPuppetServer {
    #[instrument(level = Level::DEBUG, skip(kernel))]
    pub async fn register(
        kernel: &'static Kernel,
        capacity: usize,
    ) -> Result<(), RegistrationError> {
        let (tx, rx) = KChannel::new_async(capacity).await.split();
        Ok(())
    }

    #[instrument(name = "I2cPuppetServer", level = Level::INFO, skip(kernel, rx))]
    async fn run(kernel: &'static Kernel, mut rx: KConsumer<Envelope<) {
        let mut i2c = I2cClient::from_registry(kernel).await;
    }
}
