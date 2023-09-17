//! # BBQueue
//!
//! BBQueue, short for "BipBuffer Queue", is a Single Producer Single Consumer,
//! lockless, no_std, thread safe, queue, based on [BipBuffers]. For more info on
//! the design of the lock-free algorithm used by bbqueue, see [this blog post].
//!
//! For a 90 minute guided tour of BBQueue, you can also view this [guide on YouTube].
//!
//! [guide on YouTube]: https://www.youtube.com/watch?v=ngTCf2cnGkY
//! [BipBuffers]: https://www.codeproject.com/Articles/3479/%2FArticles%2F3479%2FThe-Bip-Buffer-The-Circular-Buffer-with-a-Twist
//! [this blog post]: https://ferrous-systems.com/blog/lock-free-ring-buffer/
//!
//! BBQueue is designed (primarily) to be a First-In, First-Out queue for use with DMA on embedded
//! systems.
//!
//! While Circular/Ring Buffers allow you to send data between two threads (or from an interrupt to
//! main code), you must push the data one piece at a time. With BBQueue, you instead are granted a
//! block of contiguous memory, which can be filled (or emptied) by a DMA engine.
//!
//! ## Features
//!
//! By default BBQueue uses atomic operations which are available on most platforms. However on some
//! (mostly embedded) platforms atomic support is limited and with the default features you will get
//! a compiler error about missing atomic methods.
//!
//! This crate contains special support for Cortex-M0(+) targets with the `thumbv6` feature. By
//! enabling the feature, unsupported atomic operations will be replaced with critical sections
//! implemented by disabling interrupts. The critical sections are very short, a few instructions at
//! most, so they should make no difference to most applications.

// #![deny(missing_docs)]
// #![deny(warnings)]

mod bbbuffer;
pub use bbbuffer::*;

pub mod framed;

use core::result::Result as CoreResult;

/// Result type used by the `BBQueue` interfaces
pub type Result<T> = CoreResult<T, Error>;

/// Error type used by the `BBQueue` interfaces
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Error {
    /// The buffer does not contain sufficient size for the requested action
    InsufficientSize,

    /// Unable to produce another grant, a grant of this type is already in
    /// progress
    GrantInProgress,

    /// Unable to split the buffer, as it has already been split
    AlreadySplit,
}
