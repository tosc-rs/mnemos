//! Sermux daemons
//!
//! Daemons centered around the [serial_mux][crate::services::serial_mux] service.

use core::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    services::serial_mux::{PortHandle, WellKnown},
    Kernel,
};

//
// Sermux Loopback
//

/// Loopback Settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoopbackSettings {
    /// Should the Loopback port be enabled?
    #[serde(default)]
    pub enabled: bool,
    /// Port number. Defaults to [WellKnown::Loopback]
    #[serde(default = "LoopbackSettings::default_port")]
    pub port: u16,
    /// Buffer size, in bytes. Defaults to 128
    #[serde(default = "LoopbackSettings::default_buffer_size")]
    pub buffer_size: usize,
}

impl LoopbackSettings {
    pub const DEFAULT_PORT: u16 = WellKnown::Loopback as u16;
    pub const DEFAULT_BUFFER_SIZE: usize = 128;

    const fn default_port() -> u16 {
        Self::DEFAULT_PORT
    }
    const fn default_buffer_size() -> usize {
        Self::DEFAULT_BUFFER_SIZE
    }
}

impl Default for LoopbackSettings {
    fn default() -> Self {
        Self {
            enabled: true, // Should this default to false?
            port: Self::DEFAULT_PORT,
            buffer_size: Self::DEFAULT_BUFFER_SIZE,
        }
    }
}

/// Spawns a loopback server
///
/// Listens to all input from the given port, and echos it back
#[tracing::instrument(skip(kernel))]
pub async fn loopback(kernel: &'static Kernel, settings: LoopbackSettings) {
    let LoopbackSettings {
        port, buffer_size, ..
    } = settings;
    tracing::debug!("initializing SerMux loopback...");
    let p0 = PortHandle::open(kernel, port, buffer_size).await.unwrap();
    tracing::info!("SerMux Loopback running!");

    loop {
        let rgr = p0.consumer().read_grant().await;
        let len = rgr.len();
        tracing::trace!("Loopback read {len}B");
        p0.send(&rgr).await;
        rgr.release(len);
    }
}

//
// Sermux Hello
//

/// Hello Server Settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HelloSettings {
    /// Should the hello service be enabled?
    #[serde(default)]
    pub enabled: bool,
    /// Port number. Defaults to [WellKnown::HelloWorld]
    #[serde(default = "HelloSettings::default_port")]
    pub port: u16,
    /// Buffer size, in bytes. Defaults to 32
    #[serde(default = "HelloSettings::default_buffer_size")]
    pub buffer_size: usize,
    /// Message to print. Defaults to `b"hello\r\n"`
    #[serde(default = "HelloSettings::default_message")]
    pub message: heapless::String<32>,
    /// Interval between messages. Defaults to 1 second
    #[serde(default = "HelloSettings::default_interval")]
    pub interval: Duration,
}

impl HelloSettings {
    pub const DEFAULT_PORT: u16 = WellKnown::HelloWorld as u16;
    pub const DEFAULT_BUFFER_SIZE: usize = 32;
    pub const DEFAULT_MESSAGE_STR: &str = "hello\r\n";
    pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);

    const fn default_port() -> u16 {
        Self::DEFAULT_PORT
    }
    const fn default_buffer_size() -> usize {
        Self::DEFAULT_BUFFER_SIZE
    }
    fn default_message() -> heapless::String<32> {
        heapless::String::from(Self::DEFAULT_MESSAGE_STR)
    }
    const fn default_interval() -> Duration {
        Self::DEFAULT_INTERVAL
    }
}

impl Default for HelloSettings {
    fn default() -> Self {
        Self {
            enabled: true, // Should this default to false?
            port: Self::DEFAULT_PORT,
            buffer_size: Self::DEFAULT_BUFFER_SIZE,
            message: heapless::String::from(Self::DEFAULT_MESSAGE_STR),
            interval: Self::DEFAULT_INTERVAL,
        }
    }
}

/// Spawns a hello server
///
/// Periodically prints a message as a sign of life
#[tracing::instrument(skip(kernel))]
pub async fn hello(kernel: &'static Kernel, settings: HelloSettings) {
    let HelloSettings {
        port,
        buffer_size,
        message,
        interval,
        ..
    } = settings;
    tracing::debug!("Starting SerMux 'hello world'...");
    let p1 = PortHandle::open(kernel, port, buffer_size).await.unwrap();
    tracing::info!("SerMux 'hello world' running!");

    loop {
        kernel.sleep(interval).await;
        p1.send(message.as_bytes()).await;
    }
}
