//! # Mnemos Driver Services
//!
//! This module contains the declaration of services built in to the
//! kernel.
//!
//! In most cases, these service declarations contain:
//!
//! * The [RegisteredDriver][crate::registry::RegisteredDriver] trait implementation
//! * A definition of a client that can be used to interface with the service
//! * The Request/Response message types used by a given service
//!
//! In some cases, a service server is also provided, typically when the service
//! has no external dependencies, other than other services declared here in the
//! kernel. For an example of this, see the [serial_mux] module.

pub mod emb_display;
pub mod serial_mux;
pub mod simple_serial;
