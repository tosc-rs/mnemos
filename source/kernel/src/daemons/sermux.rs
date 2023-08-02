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
    /// Port number. Defaults to [WellKnown::Loopback]
    pub port: u16,
    /// Buffer size, in bytes. Defaults to 128
    pub buffer_size: usize,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoopbackSettingsOverrides {
    /// Should the Loopback port be enabled?
    pub enabled: bool,
    /// Port number. Defaults to [WellKnown::Loopback]
    pub port: Option<u16>,
    /// Buffer size, in bytes. Defaults to 128
    pub buffer_size: Option<usize>,
}

impl LoopbackSettings {
    const DEFAULT_PORT: u16 = WellKnown::Loopback as u16;
    const DEFAULT_BUFFER_SIZE: usize = 128;
}

impl Default for LoopbackSettings {
    fn default() -> Self {
        Self {
            port: Self::DEFAULT_PORT,
            buffer_size: Self::DEFAULT_BUFFER_SIZE,
        }
    }
}

impl LoopbackSettingsOverrides {
    pub fn into_settings(self) -> LoopbackSettings {
        LoopbackSettings {
            port: self.port.unwrap_or(LoopbackSettings::DEFAULT_PORT),
            buffer_size: self
                .buffer_size
                .unwrap_or(LoopbackSettings::DEFAULT_BUFFER_SIZE),
        }
    }
}

/// Spawns a loopback server
///
/// Listens to all input from the given port, and echos it back
#[tracing::instrument(skip(kernel))]
pub async fn loopback(kernel: &'static Kernel, settings: LoopbackSettings) {
    let LoopbackSettings { port, buffer_size } = settings;
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
    /// Port number. Defaults to [WellKnown::HelloWorld]
    pub port: u16,
    /// Buffer size, in bytes. Defaults to 32
    pub buffer_size: usize,
    /// Message to print. Defaults to `b"hello\r\n"`
    pub message: heapless::String<32>,
    /// Interval between messages. Defaults to 1 second
    pub interval: Duration,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HelloSettingsOverrides {
    /// Should the hello service be enabled?
    pub enabled: bool,
    /// Port number. Defaults to [WellKnown::HelloWorld]
    pub port: Option<u16>,
    /// Buffer size, in bytes. Defaults to 32
    pub buffer_size: Option<usize>,
    /// Message to print. Defaults to `b"hello\r\n"`
    pub message: Option<heapless::String<32>>,
    /// Interval between messages. Defaults to 1 second
    pub interval: Option<Duration>,
}

impl HelloSettingsOverrides {
    pub fn into_settings(self) -> HelloSettings {
        HelloSettings {
            port: self.port.unwrap_or(HelloSettings::DEFAULT_PORT),
            buffer_size: self
                .buffer_size
                .unwrap_or(HelloSettings::DEFAULT_BUFFER_SIZE),
            message: self
                .message
                .unwrap_or_else(|| heapless::String::from(HelloSettings::DEFAULT_MESSAGE_STR)),
            interval: self.interval.unwrap_or(HelloSettings::DEFAULT_INTERVAL),
        }
    }
}

impl HelloSettings {
    const DEFAULT_PORT: u16 = WellKnown::HelloWorld as u16;
    const DEFAULT_BUFFER_SIZE: usize = 32;
    const DEFAULT_MESSAGE_STR: &str = "hello\r\n";
    const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);
}

impl Default for HelloSettings {
    fn default() -> Self {
        Self {
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
    } = settings;
    tracing::debug!("Starting SerMux 'hello world'...");
    let p1 = PortHandle::open(kernel, port, buffer_size).await.unwrap();
    tracing::info!("SerMux 'hello world' running!");

    loop {
        kernel.sleep(interval).await;
        p1.send(message.as_bytes()).await;
    }
}
