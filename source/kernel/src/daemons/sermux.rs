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

impl Default for LoopbackSettings {
    fn default() -> Self {
        Self {
            port: WellKnown::Loopback as u16,
            buffer_size: 128,
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

impl Default for HelloSettings {
    fn default() -> Self {
        Self {
            port: WellKnown::HelloWorld as u16,
            buffer_size: 32,
            message: heapless::String::from("hello\r\n"),
            interval: Duration::from_secs(1),
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
