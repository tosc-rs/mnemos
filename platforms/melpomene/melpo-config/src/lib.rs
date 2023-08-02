//! Configuration types for Melpomene
//!
//! Separate crate so it can be used from the build.rs script

use std::time::Duration;

use mnemos_kernel::forth::Params;
use serde::{Deserialize, Serialize};

/// Melpomene configuration type
#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformConfig {
    /// TCP simulated uart driver settings
    ///
    /// If this field is None, then the tcp uart service will not
    /// be spawned.
    pub tcp_uart: Option<TcpUartConfig>,

    /// Embedded Graphics Simulator display settings
    ///
    /// If this field is None, then the display service will not
    /// be spawned.
    pub display: Option<DisplayConfig>,

    /// Forth GUI shell settings
    ///
    /// If this field is None, then the shell service will not
    /// be spawned.
    pub forth_shell: Option<ForthShell>,

    /// The maximum amount of time to sleep before repolling the
    /// executor (even if no simulated IRQs are received)
    pub sleep_cap: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TcpUartConfig {
    /// The maximum kchannel depth for processing messages
    pub kchannel_depth: usize,
    /// Incoming TCP buffer size in bytes
    pub incoming_size: usize,
    /// Outgoing TCP buffer size in bytes
    pub outgoing_size: usize,
    /// Socket addr opened as a simulated serial port
    ///
    /// For example: "127.0.0.1:9999"
    pub socket_addr: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// The maximum kchannel depth for processing messages
    pub kchannel_depth: usize,
    /// The maximum number of frames per second. Must be >= 1
    pub frames_per_second: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForthShell {
    /// IO buffer capacity in bytes
    pub capacity: usize,
    /// Forth shell parameters
    pub params: Params,
}
