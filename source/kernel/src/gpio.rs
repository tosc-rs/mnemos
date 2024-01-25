//! A generic interface for GPIO pins.
//!
//! This module provides the [`Input`] and [`Output`] traits, representing
//! digital logic GPIO pins, respectively.
//!
//! Note that this module only provides abstractions for basic GPIO operations,
//! such as setting the level of an output pin, reading the level of an input
//! pin, or waiting for an interrupt on an input pin. It does *not* contain
//! abstractions for configuring lower-level details of a GPIO pin, such as
//! integrated pull-up or pull-down resistors, output pin drive strength and
//! slew rate, or input pin interrupt sampling rate and debouncing. These
//! lower-level details are specific to the hardware platform and should be
//! configured by the platform implementation *before* using the pin with APIs
//! implemented against this module's interface.
#![deny(missing_docs)]
use core::fmt;

/// A digital logic output pin.
pub trait Output: fmt::Display {
    /// Errors returned by [`Self::try_set`].
    ///
    /// If setting the pin's output level is an infallible operation, this type
    /// may be [`core::convert::Infallible`].
    type Error: fmt::Display;

    /// Attempts to set the current output level of this pin to `level`.
    fn try_set(&mut self, level: Level) -> Result<(), Self::Error>;

    /// Sets the output level of this pin to `level`.
    ///
    /// # Panics
    ///
    /// This method calls [`Self::try_set`] and panics if it returns an error.
    fn set(&mut self, level: Level) -> &mut Self {
        match self.try_set(level) {
            Ok(_) => self,
            Err(error) => panic!("failed to set {self} output to {level}: {error}"),
        }
    }

    /// Returns the pin's current output [`Level`].
    ///
    /// This does *not* read the input level of the pin. Instead, it returns
    /// what output level the pin has currently been set to. To read digital
    /// input on a pin, use [`Input::read`], instead.
    fn output_level(&self) -> Level;

    /// Sets this pin to low.
    ///
    /// This is equivalent to `self.set(Level::Low)`.
    ///
    /// # Panics
    ///
    /// This method calls [`Self::try_set`] and panics if it returns an error.
    #[inline]
    fn set_high(&mut self) -> &mut Self {
        self.set(Level::High)
    }

    /// Sets this pin to low.
    ///
    /// This is equivalent to `self.set(Level::Low)`.
    ///
    /// # Panics
    ///
    /// This method calls [`Self::try_set`] and panics if it returns an error.
    #[inline]
    fn set_low(&mut self) -> &mut Self {
        self.set(Level::Low)
    }
}

/// A digital logic input pin.
pub trait Input: fmt::Display {
    /// Errors returned by [`Self::try_read`], [`Self::try_wait_for_level`], and
    /// [`Self::try_wait_for_edge`].
    ///
    /// If setting the pin's output level is an infallible operation, this type
    /// may be [`core::convert::Infallible`].
    type Error: fmt::Display;

    /// Attempts to return this pin's current [`Level`].
    fn try_read(&self) -> Result<Level, Self::Error>;

    /// Attempts to wait for a level-triggered interrupt on this pin.
    ///
    /// This method returns when the pin's input level transitions to the
    /// provided `level`. If the pin is already at the provided `level`, this
    /// method completes immediately. To wait for an edge-triggered interrupt
    /// instead, use [`Self::try_wait_for_edge`].
    async fn try_wait_for_level(&mut self, level: Level) -> Result<(), Self::Error>;

    /// Attempts to wait for an edge-triggered interrupt on this pin.
    ///
    /// This method returns when the pin's input level transitions on the
    /// requested [`Edge`]. To wait for a level-triggered interrupt instead, use
    /// [`Self::try_wait_for_level`].
    ///
    /// This method returns the level of the pin after the edge-triggered
    /// interrupt. This is primarily useful in conjunction with [`Edge::Any`],
    /// in which case the state of the pin cannot be assumed just because the
    /// interrupt was triggered.
    async fn try_wait_for_edge(&mut self, edge: Edge) -> Result<Level, Self::Error>;

    /// Returns this pin's current [`Level`].
    ///
    /// # Panics
    ///
    /// This method calls [`Self::try_read`] and panics if it returns an error.
    #[inline]
    fn read(&self) -> Level {
        match self.try_read() {
            Ok(level) => level,
            Err(error) => panic!("failed to read {self} input level: {error}"),
        }
    }

    /// Waits for a level-triggered interrupt on this pin.
    ///
    /// This method returns when the pin's input level transitions to the
    /// provided `level`. If the pin is already at the provided `level`, this
    /// method completes immediately. To wait for an edge-triggered interrupt
    /// instead, use [`Self::wait_for_edge`].
    ///
    /// # Panics
    ///
    /// This method calls [`Self::try_wait_for_level`] and panics if it returns an error.
    async fn wait_for_level(&mut self, level: Level) {
        if let Err(error) = self.try_wait_for_level(level).await {
            panic!("failed to wait for level-triggered ({level}) interrupt on {self}: {error}")
        }
    }

    /// Waits for an edge-triggered interrupt on this pin.
    ///
    /// This method returns when the pin's input level transitions on the
    /// requested [`Edge`]. To wait for a level-triggered interrupt instead, use
    /// [`Self::try_wait_for_level`].
    ///
    /// This method returns the level of the pin after the edge-triggered
    /// interrupt. This is primarily useful in conjunction with [`Edge::Any`],
    /// in which case the state of the pin cannot be assumed just because the
    /// interrupt was triggered.
    ///
    /// # Panics
    ///
    /// This method calls [`Self::try_wait_for_edge`] and panics if it returns an error.
    async fn wait_for_edge(&mut self, edge: Edge) -> Level {
        match self.try_wait_for_edge(edge).await {
            Ok(level) => level,
            Err(error) => {
                panic!("failed to wait for {edge} edge-triggered interrupt on {self}: {error}")
            }
        }
    }
}

/// A GPIO pin digital level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    /// Logical low.
    Low,
    /// Logical high.
    High,
}

/// A GPIO pin edge-triggered interrupt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    /// Trigger an interrupt on the rising edge (transition from low to high).
    Rising,
    /// Trigger an interrupt on the falling edge (transition from high to low).
    Falling,
    /// Trigger an interrupt on both edges (any transition).
    Any,
}

// === impl Level ===

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Level::Low => f.pad("low"),
            Level::High => f.pad("high"),
        }
    }
}

// === impl Edge ===

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Edge::Rising => f.pad("rising"),
            Edge::Falling => f.pad("falling"),
            Edge::Any => f.pad("any"),
        }
    }
}
