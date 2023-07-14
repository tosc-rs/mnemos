//! # Keyboard Service
//!
//! This module defines a generic service for modeling keyboard drivers. This
//! service can be implemented by drivers for specific keyboards, or by generic
//! "keyboard multiplexer" services. The latter is useful for systems that have
//! multiple keyboards, where a centralized service can multiplex the input from
//! multiple hardware keyboard drivers into a single stream of keyboard events
//! from all keyboards.
//!
//! The [`event`] submodule defines a generic representation of keyboard events,
//! which is, admittedly, a bit overly complex. It's intended to model as many
//! different types of keyboard as possible. Not all keyboards will provide all
//! of the available keyboard event types, based on what keys actually exist on
//! the keyboard.
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
pub struct KeyClient {
    rx: kchannel::KConsumer<KeyEvent>,
}

impl KeyClient {
    /// Obtain a `KeyClient`
    ///
    /// If the [`KeyboardService`] hasn't been registered yet, we will retry until it
    /// has been registered.
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

    pub async fn next(&mut self) -> Option<KeyEvent> {
        self.rx.dequeue_async().await.ok()
    }

    pub async fn next_char(&mut self) -> Option<char> {
        self.next().await?.into_char()
    }
}
