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

use crate::comms::kchannel;
use crate::registry::{known_uuids, RegisteredDriver};

pub mod event;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct KeyboardService;

impl RegisteredDriver for KeyboardService {
    type Request = Subscribe;
    type Response = KeySubscription;
    type Error = KeyboardError;

    const UUID: Uuid = known_uuids::kernel::KEYBOARD;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////
pub use self::event::KeyEvent;

#[derive(Debug, Eq, PartialEq)]
pub struct Subscribe {
    /// Capacity of the key subscription buffer.
    buffer_capacity: usize,
}

pub struct KeySubscription {
    rx: kchannel::KConsumer<KeyEvent>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum KeyboardError {
    NoKeyboards,
    TooManySubscriptions,
}

impl Subscribe {
    pub const DEFAULT_BUFFER_CAPACITY: usize = 32;
    pub fn with_buffer_capacity(self, buffer_capacity: usize) -> Self {
        Self { buffer_capacity }
    }
}

impl Default for Subscribe {
    fn default() -> Self {
        Self {
            buffer_capacity: Self::DEFAULT_BUFFER_CAPACITY,
        }
    }
}

impl KeySubscription {
    pub async fn new(subscription: Subscribe) -> (kchannel::KProducer<KeyEvent>, Self) {
        let Subscribe { buffer_capacity } = subscription;
        let (tx, rx) = kchannel::KChannel::new_async(buffer_capacity).await.split();
        (tx, Self { rx })
    }

    pub async fn next(&mut self) -> Option<KeyEvent> {
        self.rx.dequeue_async().await.ok()
    }

    pub async fn next_char(&mut self) -> Option<char> {
        self.next().await?.into_char()
    }
}
