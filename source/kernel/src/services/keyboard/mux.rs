use super::{KeyEvent, KeyboardError, KeyboardService, Subscribed};
use crate::{
    comms::{
        kchannel::{KChannel, KConsumer, KProducer},
        oneshot::Reusable,
    },
    mnemos_alloc::containers::FixedVec,
    registry::{
        self, known_uuids, Envelope, KernelHandle, OneshotRequestError, RegisteredDriver,
        RegistrationError,
    },
    tracing, Kernel,
};
use core::{convert::Infallible, time::Duration};
use futures::FutureExt;
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct KeyboardMuxService;

impl RegisteredDriver for KeyboardMuxService {
    type Request = Publish;
    type Response = Response;
    type Error = core::convert::Infallible;

    const UUID: Uuid = known_uuids::kernel::KEYBOARD_MUX;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Eq, PartialEq)]
pub struct Publish(KeyEvent);

pub struct Response {
    _p: (),
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct KeyboardMuxClient {
    handle: KernelHandle<KeyboardMuxService>,
    reply: Reusable<Envelope<Result<Response, Infallible>>>,
}

impl KeyboardMuxClient {
    /// Obtain a `KeyboardMuxClient`
    ///
    /// If the [`KeyboardMuxService`] hasn't been registered yet, we will retry until it
    /// has been registered.
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match Self::from_registry_no_retry(kernel).await {
                Some(port) => return port,
                None => {
                    // I2C probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain an `KeyboardMuxClient`
    ///
    /// Does NOT attempt to get an [`KeyboardMuxService`] handle more than once.
    ///
    /// Prefer [`KeyboardMuxClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let handle = kernel
            .with_registry(|reg| reg.get::<KeyboardMuxService>())
            .await?;

        Some(Self {
            handle,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn publish_key(
        &mut self,
        event: impl Into<KeyEvent>,
    ) -> Result<(), OneshotRequestError> {
        let event = event.into();
        let _ = self
            .handle
            .request_oneshot(Publish(event), &self.reply)
            .await?;
        Ok(())
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

pub struct KeyboardMuxServer {
    key_rx: KConsumer<registry::Message<KeyboardMuxService>>,
    sub_rx: KConsumer<registry::Message<KeyboardService>>,
    subscriptions: FixedVec<KProducer<KeyEvent>>,
    settings: KeyboardMuxSettings,
}

pub struct KeyboardMuxSettings {
    max_keyboards: usize,
    buffer_capacity: usize,
}

impl KeyboardMuxServer {
    /// Register the `KeyboardMuxServer`.
    pub async fn register(
        kernel: &'static Kernel,
        settings: KeyboardMuxSettings,
    ) -> Result<(), RegistrationError> {
        let (key_tx, key_rx) = KChannel::new_async(settings.buffer_capacity).await.split();
        let (sub_tx, sub_rx) = KChannel::new_async(8).await.split();
        let subscriptions = FixedVec::new(settings.max_keyboards).await;
        kernel
            .spawn(
                Self {
                    sub_rx,
                    key_rx,
                    subscriptions,
                    settings,
                }
                .run(),
            )
            .await;

        kernel
            .with_registry(|reg| {
                reg.register_konly::<KeyboardMuxService>(&key_tx)?;
                reg.register_konly::<KeyboardService>(&sub_tx)?;
                Ok(())
            })
            .await
    }

    #[tracing::instrument(level = tracing::Level::INFO, name = "KeyboardMuxServer", skip(self))]
    pub async fn run(mut self) {
        loop {
            futures::select_biased! {
                msg = self.sub_rx.dequeue_async().fuse() => {
                    let Ok(registry::Message { msg, reply }) = msg else {
                        tracing::warn!("Key subscription channel ended!");
                        break;
                    };
                    let (tx, rx) = KChannel::new_async(self.settings.buffer_capacity).await.split();
                    match self.subscriptions.try_push(tx) {
                        Ok(()) => {
                            if reply.reply_konly(msg.reply_with(Ok(Subscribed { rx }))).await.is_err() {
                                // requester is gone, so remove its subscription
                                tracing::warn!("Keyboard subscription requester is gone!");
                                self.subscriptions.pop();
                            } else {
                                tracing::info!("New keyboard subscription");
                            }
                        },
                        Err(_) => {
                            let _ = reply.reply_konly(msg.reply_with(Err(KeyboardError::TooManySubscriptions))).await;
                        }
                    }
                },
                msg = self.key_rx.dequeue_async().fuse() => {
                    let Ok(registry::Message { msg, reply }) = msg else {
                        tracing::warn!("Key publish channel ended!");
                        break;
                    };

                    let Publish(key) = msg.body;
                    tracing::debug!(?key, "publishing key event");

                    for sub in self.subscriptions.as_slice_mut() {
                        let _ = sub.enqueue_async(key).await;
                    }

                    let _ = reply.reply_konly(msg.reply_with(Ok(Response { _p: ()}))).await;
                },
            }
        }
    }
}

impl KeyboardMuxSettings {
    pub const DEFAULT_BUFFER_CAPACITY: usize = 32;
    pub const DEFAULT_MAX_KEYBOARDS: usize = 8;
}

impl Default for KeyboardMuxSettings {
    fn default() -> Self {
        Self {
            max_keyboards: Self::DEFAULT_MAX_KEYBOARDS,
            buffer_capacity: Self::DEFAULT_BUFFER_CAPACITY,
        }
    }
}
