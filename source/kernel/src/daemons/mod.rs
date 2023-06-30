//! Daemons
//!
//! This module contains tasks that run in the background and perform
//! some ongoing responsibility.
//!
//! Unlike [services][crate::services], daemons are not exposed as a
//! client/server via the [registry][crate::registry].

use core::time::Duration;

use crate::{
    services::serial_mux::{PortHandle, WellKnown},
    tracing, Kernel,
};

//
// Sermux Loopback
//

/// Loopback Settings
#[derive(Debug, Clone)]
pub struct SermuxLoopbackSettings {
    /// Port number. Defaults to [WellKnown::Loopback]
    pub port: u16,
    /// Buffer size, in bytes. Defaults to 128
    pub buffer_size: usize,
}

impl Default for SermuxLoopbackSettings {
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
pub async fn sermux_loopback(kernel: &'static Kernel, settings: SermuxLoopbackSettings) {
    let SermuxLoopbackSettings { port, buffer_size } = settings;
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
#[derive(Debug, Clone)]
pub struct SermuxHelloSettings {
    /// Port number. Defaults to [WellKnown::Loopback]
    pub port: u16,
    /// Buffer size, in bytes. Defaults to 32
    pub buffer_size: usize,
    /// Message to print. Defaults to `b"hello\r\n"`
    pub message: &'static [u8],
    /// Interval between messages. Defaults to 1 second
    pub interval: Duration,
}

impl Default for SermuxHelloSettings {
    fn default() -> Self {
        Self {
            port: WellKnown::HelloWorld as u16,
            buffer_size: 32,
            message: b"hello\r\n",
            interval: Duration::from_secs(1),
        }
    }
}

/// Spawns a hello server
///
/// Periodically prints a message as a sign of life
#[tracing::instrument(skip(kernel))]
pub async fn sermux_hello(kernel: &'static Kernel, settings: SermuxHelloSettings) {
    let SermuxHelloSettings {
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
        p1.send(message).await;
    }
}
