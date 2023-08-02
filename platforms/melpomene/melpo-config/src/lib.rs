//! Configuration types for Melpomene
//!
//! Separate crate so it can be used from the build.rs script

use std::{net::SocketAddr, time::Duration};

use mnemos_kernel::forth::{Params, ParamsOverrides};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformConfig {
    /// TCP simulated uart driver settings
    ///
    /// If this field is None, then the tcp uart service will not
    /// be spawned.
    pub tcp_uart: TcpUartConfigOverrides,

    /// Embedded Graphics Simulator display settings
    ///
    /// If this field is None, then the display service will not
    /// be spawned.
    pub display: DisplayConfigOverrides,

    /// Forth GUI shell settings
    ///
    /// If this field is None, then the shell service will not
    /// be spawned.
    pub forth_shell: ForthShellOverrides,

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
    /// The maximum kchannel depth for processing messages
    pub kchannel_depth: usize,
    /// Incoming TCP buffer size in bytes
    pub incoming_size: usize,
    /// Outgoing TCP buffer size in bytes
    pub outgoing_size: usize,
    /// Socket addr opened as a simulated serial port
    ///
    /// For example: "127.0.0.1:9999"
    pub socket_addr: SocketAddr,
}

impl TcpUartConfig {
    const DEFAULT_KCHANNEL_DEPTH: usize = 2;
    const DEFAULT_INCOMING_SIZE: usize = 4096;
    const DEFAULT_OUTGOING_SIZE: usize = 4096;
    const DEFAULT_SOCKET_ADDR_STR: &str = "127.0.0.1:9999";
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TcpUartConfigOverrides {
    /// Should the TCP UART be enabled?
    pub enabled: bool,
    /// The maximum kchannel depth for processing messages
    pub kchannel_depth: Option<usize>,
    /// Incoming TCP buffer size in bytes
    pub incoming_size: Option<usize>,
    /// Outgoing TCP buffer size in bytes
    pub outgoing_size: Option<usize>,
    /// Socket addr opened as a simulated serial port
    ///
    /// For example: Option<"127>.0.0.1:9999"
    pub socket_addr: Option<SocketAddr>,
}

impl TcpUartConfigOverrides {
    pub fn into_settings(self) -> TcpUartConfig {
        TcpUartConfig {
            kchannel_depth: self
                .kchannel_depth
                .unwrap_or(TcpUartConfig::DEFAULT_KCHANNEL_DEPTH),
            incoming_size: self
                .incoming_size
                .unwrap_or(TcpUartConfig::DEFAULT_INCOMING_SIZE),
            outgoing_size: self
                .outgoing_size
                .unwrap_or(TcpUartConfig::DEFAULT_OUTGOING_SIZE),
            socket_addr: self
                .socket_addr
                .unwrap_or_else(|| TcpUartConfig::DEFAULT_SOCKET_ADDR_STR.parse().unwrap()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// The maximum kchannel depth for processing messages
    pub kchannel_depth: usize,
    /// The maximum number of frames per second. Must be >= 1
    pub frames_per_second: usize,
}

impl DisplayConfig {
    const DEFAULT_KCHANNEL_DEPTH: usize = 2;
    const DEFAULT_FRAMES_PER_SECOND: usize = 20;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfigOverrides {
    /// Should the display be enabled?
    pub enabled: bool,
    /// The maximum kchannel depth for processing messages
    pub kchannel_depth: Option<usize>,
    /// The maximum number of frames per second. Must be >= 1
    pub frames_per_second: Option<usize>,
}

impl DisplayConfigOverrides {
    pub fn into_settings(self) -> DisplayConfig {
        DisplayConfig {
            kchannel_depth: self
                .kchannel_depth
                .unwrap_or(DisplayConfig::DEFAULT_KCHANNEL_DEPTH),
            frames_per_second: self
                .frames_per_second
                .unwrap_or(DisplayConfig::DEFAULT_FRAMES_PER_SECOND),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForthShell {
    /// IO buffer capacity in bytes
    pub capacity: usize,
    /// Forth shell parameters
    pub params: Params,
}

impl ForthShell {
    const DEFAULT_CAPACITY: usize = 1024;
    const DEFAULT_PARAMS: Params = Params::new();
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForthShellOverrides {
    pub enabled: bool,
    /// IO buffer capacity in bytes
    pub capacity: Option<usize>,
    /// Forth shell parameters
    pub params: Option<ParamsOverrides>,
}

impl ForthShellOverrides {
    pub fn into_settings(self) -> ForthShell {
        ForthShell {
            capacity: self.capacity.unwrap_or(ForthShell::DEFAULT_CAPACITY),
            params: self
                .params
                .map(ParamsOverrides::into_settings)
                .unwrap_or(ForthShell::DEFAULT_PARAMS),
        }
    }
}
