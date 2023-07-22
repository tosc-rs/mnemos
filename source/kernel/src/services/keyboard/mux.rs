//! Keyboard multiplexer service.
//!
//! This module contains the [`KeyboardMuxServer`] type, which implements both
//! [`KeyboardService`] and a [`KeyboardMuxService`] defined in this module. It
//! is used for systems where multiple hardware keyboards may be available, to
//! allow clients to subscribe to events from *any keyboard* (using its
//! [`KeyboardService`] implementation). Keyboard drivers use the
//! [`KeyboardMuxService`] to publish events from their keyboards to the
//! multiplexer, which broadcasts those events to all clients.
use super::{key_event, KeyEvent, KeyboardError, KeyboardService, Subscribed};
use crate::{
    comms::{
        bbq,
        kchannel::{KChannel, KConsumer, KProducer},
        oneshot::Reusable,
    },
    mnemos_alloc::containers::FixedVec,
    registry::{
        self, known_uuids, Envelope, KernelHandle, OneshotRequestError, RegisteredDriver,
        RegistrationError,
    },
    services::serial_mux,
    tracing::{self, Level},
    Kernel,
};
use core::{convert::Infallible, time::Duration};
use futures::{future, FutureExt};
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// Service definition for the keyboard multiplexer.
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

/// A client for the [`KeyboardMuxService`].
///
/// This type is used by keyboard drivers to broadcast events from their
/// hardware keyboard to the [`KeyboardMuxService`]. It is obtained using
/// [`KeyboardMuxClient::from_registry`].
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

/// The keyboard multiplexer.
///
/// This type implements both [`KeyboardMuxService`] *and* [`KeyboardService`].
/// The [`KeyboardMuxService`] implementation is used by keyboard drivers to
/// publish their key events to the multiplexer, while the [`KeyboardService`]
/// implementation is used for tasks that consume keyboard input to subscribe to
/// key events.
pub struct KeyboardMuxServer {
    key_rx: KConsumer<registry::Message<KeyboardMuxService>>,
    sub_rx: KConsumer<registry::Message<KeyboardService>>,
    subscriptions: FixedVec<KProducer<KeyEvent>>,
    settings: KeyboardMuxSettings,
    sermux_port: Option<serial_mux::PortHandle>,
}

#[derive(Debug)]
pub struct KeyboardMuxSettings {
    max_keyboards: usize,
    buffer_capacity: usize,
    sermux_port: Option<u16>,
}

impl KeyboardMuxServer {
    /// Register the `KeyboardMuxServer`.
    ///
    /// If [`KeyboardMuxSettings::with_sermux_port`] is [`Some`], this function
    /// will attempt to acquire a [`serial_mux::PortHandle`] for the configured
    /// serial mux port.
    #[tracing::instrument(
        name = "KeyboardMuxServer::register",
        level = Level::DEBUG,
        skip(kernel),
        err(Debug),
    )]
    pub async fn register(
        kernel: &'static Kernel,
        settings: KeyboardMuxSettings,
    ) -> Result<(), RegistrationError> {
        let (key_tx, key_rx) = KChannel::new_async(settings.buffer_capacity).await.split();
        let (sub_tx, sub_rx) = KChannel::new_async(8).await.split();
        let subscriptions = FixedVec::new(settings.max_keyboards).await;
        let sermux_port = if let Some(port) = settings.sermux_port {
            let mut client = serial_mux::SerialMuxClient::from_registry(kernel).await;
            tracing::info!("opening Serial Mux port {port}");
            Some(
                client
                    .open_port(port, settings.buffer_capacity)
                    .await
                    // TODO(eliza): this could be a custom RegistrationError variant...
                    .expect("failed to acquire serial mux keyboard port!"),
            )
        } else {
            None
        };
        kernel
            .spawn(
                Self {
                    sub_rx,
                    key_rx,
                    subscriptions,
                    settings,
                    sermux_port,
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
            .await?;
        tracing::info!("KeyboardMuxServer registered!");
        Ok(())
    }

    #[tracing::instrument(name = "KeyboardMuxServer", level = Level::INFO, skip(self))]
    pub async fn run(mut self) {
        loop {
            let sermux_fut = match self.sermux_port {
                Some(ref mut port) => {
                    let rgr = port.consumer().read_grant();
                    future::Either::Left(rgr)
                }
                None => future::Either::Right(future::pending::<bbq::GrantR>()),
            };
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
                rgr = sermux_fut.fuse() => {
                    let len = rgr.len();
                    for &byte in &rgr[..] {
                        let Some(key) = KeyEvent::from_ascii(byte, key_event::Kind::Pressed) else {
                            tracing::warn!("invalid ASCII byte on SerMux port: {byte:#x}");
                            continue;
                        };
                        tracing::debug!(?key, "publishing SerMux key event");

                        for sub in self.subscriptions.as_slice_mut() {
                            let _ = sub.enqueue_async(key).await;
                        }
                    }
                    rgr.release(len);
                }

            }
        }
    }
}

impl KeyboardMuxSettings {
    pub const DEFAULT_BUFFER_CAPACITY: usize = 32;
    pub const DEFAULT_MAX_KEYBOARDS: usize = 8;
    pub const DEFAULT_SERMUX_PORT: Option<u16> = Some(serial_mux::WellKnown::PseudoKeyboard as u16);

    /// Sets a [serial mux](crate::services::serial_mux) port to use as a
    /// virtual keyboard input.
    ///
    /// If this is [`None`], serial mux input will not be used as a virtual
    /// keyboard.
    #[must_use]
    pub fn with_sermux_port(self, port: impl Into<Option<u16>>) -> Self {
        let sermux_port = port.into();
        Self {
            sermux_port,
            ..self
        }
    }
}

impl Default for KeyboardMuxSettings {
    fn default() -> Self {
        Self {
            max_keyboards: Self::DEFAULT_MAX_KEYBOARDS,
            buffer_capacity: Self::DEFAULT_BUFFER_CAPACITY,
            sermux_port: Self::DEFAULT_SERMUX_PORT,
        }
    }
}
