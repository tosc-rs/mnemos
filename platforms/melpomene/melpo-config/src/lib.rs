//! Configuration types for Melpomene
//!
//! Separate crate so it can be used from the build.rs script

use std::{net::SocketAddr, time::Duration};

use mnemos_kernel::forth::Params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformConfig {
    /// TCP simulated uart driver settings
    ///
    /// If this field is None, then the tcp uart service will not
    /// be spawned.
    pub tcp_uart: TcpUartConfig,

    /// Embedded Graphics Simulator display settings
    ///
    /// If this field is None, then the display service will not
    /// be spawned.
    pub display: DisplayConfig,

    /// Forth GUI shell settings
    ///
    /// If this field is None, then the shell service will not
    /// be spawned.
    pub forth_shell: ForthShell,

    /// The maximum amount of time to sleep before repolling the
    /// executor (even if no simulated IRQs are received)
    pub sleep_cap: Option<Duration>,
}

impl PlatformConfig {
    pub const fn default_sleep_cap() -> Duration {
        Duration::from_millis(100)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TcpUartConfig {
    /// Should the TCP UART be enabled?
    #[serde(default)]
    pub enabled: bool,
    /// The maximum kchannel depth for processing messages
    #[serde(default = "TcpUartConfig::default_kchannel_depth")]
    pub kchannel_depth: usize,
    /// Incoming TCP buffer size in bytes
    #[serde(default = "TcpUartConfig::default_incoming_size")]
    pub incoming_size: usize,
    /// Outgoing TCP buffer size in bytes
    #[serde(default = "TcpUartConfig::default_outgoing_size")]
    pub outgoing_size: usize,
    /// Socket addr opened as a simulated serial port
    ///
    /// For example: "127.0.0.1:9999"
    #[serde(default = "TcpUartConfig::default_socket_addr")]
    pub socket_addr: SocketAddr,
}

impl TcpUartConfig {
    pub const DEFAULT_KCHANNEL_DEPTH: usize = 2;
    pub const DEFAULT_INCOMING_SIZE: usize = 4096;
    pub const DEFAULT_OUTGOING_SIZE: usize = 4096;
    pub const DEFAULT_SOCKET_ADDR_STR: &str = "127.0.0.1:9999";

    const fn default_kchannel_depth() -> usize {
        Self::DEFAULT_KCHANNEL_DEPTH
    }
    const fn default_incoming_size() -> usize {
        Self::DEFAULT_INCOMING_SIZE
    }
    const fn default_outgoing_size() -> usize {
        Self::DEFAULT_OUTGOING_SIZE
    }
    fn default_socket_addr() -> SocketAddr {
        Self::DEFAULT_SOCKET_ADDR_STR.parse().unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// Should the display be enabled
    #[serde(default)]
    pub enabled: bool,
    /// The maximum kchannel depth for processing messages
    #[serde(default = "DisplayConfig::default_kchannel_depth")]
    pub kchannel_depth: usize,
    /// The maximum number of frames per second. Must be >= 1
    #[serde(default = "DisplayConfig::default_frames_per_second")]
    pub frames_per_second: usize,
}

impl DisplayConfig {
    pub const DEFAULT_KCHANNEL_DEPTH: usize = 2;
    pub const DEFAULT_FRAMES_PER_SECOND: usize = 20;

    const fn default_kchannel_depth() -> usize {
        Self::DEFAULT_KCHANNEL_DEPTH
    }
    const fn default_frames_per_second() -> usize {
        Self::DEFAULT_FRAMES_PER_SECOND
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForthShell {
    /// Should the forth shell be enabled
    #[serde(default)]
    pub enabled: bool,
    /// IO buffer capacity in bytes
    #[serde(default = "ForthShell::default_capacity")]
    pub capacity: usize,
    /// Forth shell parameters
    #[serde(default = "ForthShell::default_params")]
    pub params: Params,
}

impl ForthShell {
    pub const DEFAULT_CAPACITY: usize = 1024;
    pub const DEFAULT_PARAMS: Params = Params::new();

    const fn default_capacity() -> usize {
        Self::DEFAULT_CAPACITY
    }
    const fn default_params() -> Params {
        Self::DEFAULT_PARAMS
    }
}
