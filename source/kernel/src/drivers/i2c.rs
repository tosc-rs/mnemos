//! # I<sup>2</sup>C
//!
//! A service definition for communicating with I<sup>2</sup>C devices.
//!
//! This module only contains the service definition and client definition,
//! the server must be implemented for the given target platform's
//! I<sup>2</sup>C hardware.

use uuid::Uuid;

use crate::comms::bbq::BidiHandle;
use crate::comms::oneshot::Reusable;
use crate::Kernel;

use crate::registry::{known_uuids, Envelope, KernelHandle, RegisteredDriver, ReplyTo};
pub use embedded_hal_async::i2c::*;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// `I2CService` is the registered driver type
pub struct I2CService;

impl RegisteredDriver for I2CService {
    type Request = Request;
    type Response = Response;
    type Error = I2CError;
    const UUID: Uuid = crate::registry::known_uuids::kernel::I2C;
}

pub enum Request {
    /// Open a connection with the I<sup>2</sup>C device at the provided 7-bit
    /// address.
    Open(SevenBitAddress),
    /// Open a connection with the I<sup>2</sup>C device at the provided 7-bit
    /// address.
    OpenTenBit(TenBitAddress),
}