//! System Call Types and low level methods
//!
//! These system call types act as the "wire format" between the kernel
//! and userspace.
//!
//! These types and functions are NOT generally used outside of creating
//! [`porcelain`](crate::porcelain) functions, or within the kernel itself.
//!
//! Consider using the [`porcelain`](crate::porcelain) functions directly
//! instead when making userspace system calls.
//!
//! ## WARNING!
//!
//! Care must be taken when modifying these types! Non-additive changes,
//! including ANY field reordering **MUST** be considered a breaking change!
//!
//! I have chosen NOT to mark these enums as `#[non_exhaustive]` as
//! Serde will already fail deserialization on an unknown enum variant.
//!
//! Breakages of downstream code causing non-exhaustive enum matching errors
//! due to added enum variants are NOT considered a "breaking change" at the
//! moment. If this is important to you, pin the exact `common` crate version
//! you plan to support, or open an issue to discuss changing this policy.

pub mod serial;

use serde::{Serialize, Deserialize};

// This is SUPPOSED to be used to route incoming userspace requests to the proper
// kernelspace driver. I'm not sure this is the right abstraction.
//
// TODO: This MUST be kept in sync with UserRequestBody!
#[derive(PartialEq, Eq)]
pub enum DriverKind {
    Serial,

    // I'm not sure if I actually want to keep the "driverkind" paradigm.
    Todo,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct UserRequest {
    pub header: UserRequestHeader,
    pub body: UserRequestBody,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct UserRequestHeader {
    pub nonce: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum UserRequestBody {
    Serial(serial::SerialRequest),
}

impl UserRequest {
    pub fn driver_kind(&self) -> DriverKind {
        match self.body {
            UserRequestBody::Serial(_) => DriverKind::Serial,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum KernelMsg {
    Timestamp(u64),
    Dealloc(ByteBoxWire),
    Response(KernelResponse),
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct KernelResponse {
    pub header: KernelResponseHeader,
    pub body: KernelResponseBody,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct KernelResponseHeader {
    pub nonce: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum KernelResponseBody {
    Serial(Result<serial::SerialResponse, serial::SerialError>),
    TodoLoopback,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct ByteBoxWire {
    pub ptr: usize,
    pub len: usize,
}

