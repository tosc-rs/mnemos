//! # Keyboard Service
//!
//! This module defines a generic service for modeling keyboard drivers. This
//! service can be implemented by drivers for specific keyboards, or by a generic
//! ["keyboard multiplexer" service](self::mux::KeyboardMuxService) type in the
//! [`mux`] submodule. The latter is useful for systems that have multiple
//! keyboards, as it allows clients which consume keyboard input to subscribe to
//! events from *all* hardware keyboard drivers, rather than a single keyboard.
//!
//! The [`key_event`] submodule defines a generic representation of keyboard
//! events, which is, admittedly, a bit overly complex. It's intended to model
//! as many different types of keyboard as possible. Not all keyboards will
//! provide all of the available keyboard event types, based on what keys
//! actually exist on the keyboard.
use uuid::Uuid;

use crate::{
    comms::{
        kchannel::{self, KChannel},
        oneshot,
    },
    registry::{known_uuids, RegisteredDriver},
    Kernel,
};
use core::time::Duration;

pub mod key_event;
pub mod mux;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct KeyboardService;

impl RegisteredDriver for KeyboardService {
    type Request = Subscribe;
    type Response = Subscribed;
    type Error = KeyboardError;

    const UUID: Uuid = known_uuids::kernel::KEYBOARD;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////
pub use self::key_event::KeyEvent;

#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Subscribe {
    buffer_capacity: usize,
}

pub struct Subscribed {
    rx: kchannel::KConsumer<KeyEvent>,
}

#[derive(Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum KeyboardError {
    NoKeyboards,
    TooManySubscriptions,
}

#[derive(Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum KeyClientError {
    NoKeyboardService,
}

impl Default for Subscribe {
    fn default() -> Self {
        Self {
            buffer_capacity: Self::DEFAULT_BUFFER_CAPACITY,
        }
    }
}

impl Subscribe {
    pub const DEFAULT_BUFFER_CAPACITY: usize = 32;

    pub fn with_buffer_capacity(self, buffer_capacity: usize) -> Self {
        Self { buffer_capacity }
    }
}

impl Subscribed {
    pub fn new(Subscribe { buffer_capacity }: Subscribe) -> (kchannel::KProducer<KeyEvent>, Self) {
        let (tx, rx) = KChannel::new(buffer_capacity).split();
        (tx, Self { rx })
    }
}

////////////////////////////////////////////////////////////////////////////////
// Client types
////////////////////////////////////////////////////////////////////////////////

/// A client that receives [`KeyEvent`]s from a [`KeyboardService`].
pub struct KeyClient {
    rx: kchannel::KConsumer<KeyEvent>,
}

impl KeyClient {
    /// Obtain a `KeyClient`
    ///
    /// If the [`KeyboardService`] hasn't been registered yet, we will retry until it
    /// has been registered.
    #[must_use]
    pub async fn from_registry(kernel: &'static Kernel, subscribe: Subscribe) -> Self {
        loop {
            match Self::from_registry_no_retry(kernel, subscribe).await {
                Some(port) => return port,
                None => {
                    // I2C probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain an `KeyClient`
    ///
    /// Does NOT attempt to get an [`KeyboardService`] handle more than once.
    ///
    /// Prefer [`KeyClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    #[must_use]
    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
        subscribe: Subscribe,
    ) -> Option<Self> {
        let mut handle = kernel
            .with_registry(|reg| reg.get::<KeyboardService>())
            .await?;
        let reply = oneshot::Reusable::new_async().await;
        let Subscribed { rx } = handle
            .request_oneshot(subscribe, &reply)
            .await
            .ok()?
            .body
            .ok()?;
        Some(Self { rx })
    }

    /// Returns the next [`KeyEvent`] received from the [`KeyboardService`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`KeyEvent`]`)` when a keyboard event is received.
    /// - [`Err`]`(`[`KeyClientError`]`)` if the [`KeyboardService`] is no
    ///   longer available.
    pub async fn next(&mut self) -> Result<KeyEvent, KeyClientError> {
        self.rx
            .dequeue_async()
            .await
            .map_err(|_| KeyClientError::NoKeyboardService)
    }

    /// Returns the next Unicode [`char`] received from the [`KeyboardService`].
    ///
    /// Any [`KeyEvent`]s which are not Unicode [`char`]s are skipped. This
    /// method is equivalent to calling [`KeyEvent::into_char`] on the
    /// [`KeyEvent`] returned by [`KeyClient::next`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`char`]`)` if a keyboard event is received and the keyboard
    ///   event corresponds to a Unicode [`char`]
    /// - [`Err`]`(`[`KeyClientError`]`)` if the [`KeyboardService`] is no
    ///   longer available.
    pub async fn next_char(&mut self) -> Result<char, KeyClientError> {
        loop {
            // if the key subscription stream has ended, return `None`.
            let key = self.next().await?;

            // if the next event is a char, return it. otherwise, keep
            // waiting for the next event
            if let Some(c) = key.into_char() {
                return Ok(c);
            }
        }
    }
}
