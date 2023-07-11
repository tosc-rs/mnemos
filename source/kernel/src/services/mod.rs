//! # Mnemos Services
//!
//! This module contains the declaration of services built in to the
//! kernel.
//!
//! In most cases, these service declarations contain:
//!
//! * A **Service**
//!     * This is the [RegisteredDriver][crate::registry::RegisteredDriver] trait implementation
//!     * Generally should be an empty/ZST struct
//!     * Also includes the Request/Response message types used by a given service
//! * A **Client**
//!     * The definition of a client that can be used to interface with the service
//! * *Optionally*, a **Server**
//!     * typically only when the service has no external dependencies, other than
//!       other services declared here in the kernel. For an example of this, see
//!       the [serial_mux] module.
//!
//! For examples of using these services, see the [daemons][crate::daemons] module.

pub mod emb_display;
pub mod forth_spawnulator;
pub mod i2c;
pub mod serial_mux;
pub mod simple_serial;
