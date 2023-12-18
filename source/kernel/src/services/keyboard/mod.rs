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
    registry::{self, known_uuids, KernelHandle, UserService},
    Kernel,
};

pub mod key_event;
pub mod mux;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct KeyboardService;

impl UserService for KeyboardService {
    type ClientMsg = ();
    type ServerMsg = KeyEvent;
    type Hello = Subscribe;
    type ConnectError = KeyboardError;

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

////////////////////////////////////////////////////////////////////////////////
// Client types
////////////////////////////////////////////////////////////////////////////////

/// A client that receives [`KeyEvent`]s from a [`KeyboardService`].
pub struct KeyClient {
    handle: KernelHandle<KeyboardService>,
}

#[derive(Debug)]
pub enum FromRegistryError {
    Connect(registry::ConnectError<KeyboardService>),
    Service(KeyboardError),
}

impl KeyClient {
    /// Obtain a `KeyClient`
    ///
    /// If the [`KeyboardService`] hasn't been registered yet, we will retry until it
    /// has been registered.
    pub async fn from_registry(
        kernel: &'static Kernel,
        subscribe: Subscribe,
    ) -> Result<Self, FromRegistryError> {
        let handle = kernel
            .registry()
            .connect::<KeyboardService>(subscribe)
            .await
            .map_err(FromRegistryError::Connect)?;
        Self::from_handle(subscribe, handle).await
    }

    /// Obtain an `KeyClient`
    ///
    /// Does NOT attempt to get an [`KeyboardService`] handle more than once.
    ///
    /// Prefer [`KeyClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
        subscribe: Subscribe,
    ) -> Result<Self, FromRegistryError> {
        let handle = kernel
            .registry()
            .try_connect::<KeyboardService>(())
            .await
            .map_err(FromRegistryError::Connect)?;
        Self::from_handle(subscribe, handle).await
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
