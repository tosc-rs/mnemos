use bbq10kbd::{AsyncBbq10Kbd, CapsLockState, FifoCount};
pub use bbq10kbd::{KeyRaw, KeyStatus, Version};
use core::time::Duration;
use kernel::{
    comms::{
        kchannel::{KChannel, KConsumer, KProducer},
        oneshot::{self, Reusable},
    },
    embedded_hal_async::i2c::{self, I2c},
    mnemos_alloc::containers::FixedVec,
    registry::{self, Envelope, KernelHandle, RegisteredDriver},
    services::i2c::{I2cClient, I2cError},
    trace::{self, instrument, Instrument, Level},
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

#[derive(Debug)]
pub enum Request {
    GetVersion,
    SetColor(RgbColor),
    SetBacklight(u8),
    ToggleLed(bool),
    GetLedStatus,
    SubscribeToKeys,
}

pub enum Response {
    GetVersion(Version),
    SetColor(RgbColor),
    SetBacklight(u8),
    ToggleLed(bool),
    GetLedStatus { color: RgbColor, on: bool },
    SubscribeToKeys(KeySubscription),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug)]
pub enum Error {
    I2c(I2cError),
    AtMaxSubscriptions,
}

pub struct KeySubscription(KConsumer<(KeyStatus, KeyRaw)>);

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct I2cPuppetClient {
    handle: KernelHandle<I2cPuppetService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

impl I2cPuppetClient {
    /// Obtain an `I2cPuppetClient`
    ///
    /// If the [`I2cPuppetService`] hasn't been registered yet, we will retry until it
    /// has been registered.
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match I2cPuppetClient::from_registry_no_retry(kernel).await {
                Some(port) => return port,
                None => {
                    // I2C probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain an `I2cPuppetClient`
    ///
    /// Does NOT attempt to get an [`I2cPuppetService`] handle more than once.
    ///
    /// Prefer [`I2cPuppetClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let handle = kernel
            .with_registry(|reg| reg.get::<I2cPuppetService>())
            .await?;

        Some(I2cPuppetClient {
            handle,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn subscribe_to_keys(&mut self) -> Result<KeySubscription, Error> {
        let resp = self
            .handle
            .request_oneshot(Request::SubscribeToKeys, &self.reply)
            .await
            .unwrap();
        if let Response::SubscribeToKeys(sub) = resp.body? {
            Ok(sub)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

/// Server implementation for the [`I2cPuppetService`].
pub struct I2cPuppetServer {
    settings: I2cPuppetSettings,
    rx: KConsumer<registry::Message<I2cPuppetService>>,
    i2c: I2cClient,
    subscriptions: FixedVec<KProducer<(KeyStatus, KeyRaw)>>,
}

#[derive(Debug)]
pub enum RegistrationError {
    Registry(registry::RegistrationError),
    NoI2cPuppet(I2cError),
}

impl I2cPuppetServer {
    #[instrument(level = Level::DEBUG, skip(kernel))]
    pub async fn register(
        kernel: &'static Kernel,
        settings: I2cPuppetSettings,
    ) -> Result<(), RegistrationError> {
        let (tx, rx) = KChannel::new_async(settings.channel_capacity).await.split();
        let mut i2c = I2cClient::from_registry(kernel).await;
        let subscriptions = FixedVec::new(settings.max_subscriptions).await;
        {
            // first, make sure we can get the version...
            let mut kbd = AsyncBbq10Kbd::new(&mut i2c);
            let Version { major, minor } = kbd
                .get_version()
                .await
                .map_err(RegistrationError::NoI2cPuppet)?;
            tracing::info!("i2c_puppet firmware version: v{major}.{minor}");
            // // and then, reset i2c_puppet
            // kbd.sw_reset()
            //     .await
            //     .map_err(RegistrationError::NoI2cPuppet)?;
            // kernel.timer().sleep(Duration::from_millis(20)).await;
            // tracing::info!("i2c_puppet reset");
        }
        let this = Self {
            settings,
            rx,
            i2c,
            subscriptions,
        };

        kernel
            .spawn(
                async move {
                    if let Err(error) = this.run(kernel).await {
                        tracing::error!(%error, "i2c_puppet server terminating on fatal error!");
                    }
                }
                .instrument(trace::info_span!("I2cPuppetServer")),
            )
            .await;

        kernel
            .with_registry(|reg| reg.register_konly::<I2cPuppetService>(&tx))
            .await
            .map_err(RegistrationError::Registry)?;
        Ok(())
    }

    async fn run(mut self, kernel: &'static Kernel) -> Result<(), I2cError> {
        loop {
            // XXX(eliza): this Sucks and we should instead get i2c_puppet to send
            // us an interrupt...
            if let Ok(dq) = kernel
                .timer()
                .timeout(self.settings.poll_interval, self.rx.dequeue_async())
                .await
            {
                let registry::Message { msg, reply } = match dq {
                    Ok(msg) => msg,
                    Err(_) => return Ok(()),
                };
                match msg.body {
                    Request::SubscribeToKeys => {
                        let (sub_tx, sub_rx) =
                            KChannel::new_async(self.settings.subscription_capacity)
                                .await
                                .split();
                        match self.subscriptions.try_push(sub_tx) {
                            Ok(()) => {
                                tracing::info!("new subscription to keys");
                                reply
                                    .reply_konly(msg.reply_with(Ok(Response::SubscribeToKeys(
                                        KeySubscription(sub_rx),
                                    ))))
                                    .await;
                            }
                            Err(_) => {
                                tracing::warn!("subscriptions at capacity");
                                reply
                                    .reply_konly(msg.reply_with(Err(Error::AtMaxSubscriptions)))
                                    .await;
                            }
                        }
                    }
                    req => todo!("eliza: {req:?}"),
                }
            }

            if !self.subscriptions.is_empty() {
                tracing::trace!("polling keys...");
                self.poll_keys().await?;
            }
        }
    }

    async fn poll_keys(&mut self) -> Result<(), I2cError> {
        let mut kbd = AsyncBbq10Kbd::new(&mut self.i2c);
        loop {
            let status = kbd.get_key_status().await?;
            if let FifoCount::Known(0) = status.fifo_count {
                break;
            }
            let key = kbd.get_fifo_key_raw().await?;
            trace::debug!(?key);
            // TODO(eliza): remove dead subscriptions...
            for sub in self.subscriptions.as_slice_mut() {
                if let Err(error) = sub.enqueue_async((status, key)).await {
                    trace::warn!(?error, "subscription dropped...");
                }
            }
        }
        Ok(())
    }
}

// === I2cPuppetSettings ===

#[derive(Debug)]
pub struct I2cPuppetSettings {
    pub channel_capacity: usize,
    pub subscription_capacity: usize,
    pub max_subscriptions: usize,
    pub poll_interval: Duration,
}

impl Default for I2cPuppetSettings {
    fn default() -> Self {
        Self {
            channel_capacity: 8,
            subscription_capacity: 32,
            max_subscriptions: 8,
            poll_interval: Duration::from_secs(1),
        }
    }
}

// === impl KeySubscription ===

pub enum KeySubscriptionError {
    Closed,
    Decode,
    InvalidKey,
}

impl KeySubscription {
    pub async fn next_char(&mut self) -> Result<char, KeySubscriptionError> {
        loop {
            let (status, key) = self.next_raw().await?;
            let x = match key {
                KeyRaw::Pressed(x) => x,
                // KeyRaw::Released(x) => x,
                KeyRaw::Invalid => return Err(KeySubscriptionError::InvalidKey),
                _ => continue,
            };
            if let Some(mut c) = char::from_u32(x as u32) {
                if status.caps_lock == CapsLockState::On {
                    c = c.to_ascii_uppercase();
                }
                return Ok(c);
            } else {
                return Err(KeySubscriptionError::Decode);
            }
        }
    }

    pub async fn next_raw(&mut self) -> Result<(KeyStatus, KeyRaw), KeySubscriptionError> {
        self.0
            .dequeue_async()
            .await
            .map_err(|_| KeySubscriptionError::Closed)
    }
}
