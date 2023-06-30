//! Daemons
//!
//! This module contains tasks that run in the background and perform
//! some ongoing responsibility.
//!
//! Unlike [services][crate::services], daemons are not exposed as a
//! client/server via the [registry][crate::registry].

pub mod sermux;
pub mod shells;
