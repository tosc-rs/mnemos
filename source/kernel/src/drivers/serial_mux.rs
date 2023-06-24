//! # Serial Multiplexor
//!
//! Allows the creation of virtual "ports" over a single serial link
//!
//! This module includes the service definition, client definition, as well
//! as a server definition that relies on the [`SimpleSerial`][crate::drivers::simple_serial]
//! service to provide the service implementation.

use core::time::Duration;

use crate::tracing::{debug, warn};
use crate::{
    comms::{
        bbq,
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    drivers::simple_serial::SimpleSerialClient,
    registry::{Envelope, KernelHandle, Message, RegisteredDriver},
    Kernel,
};
use maitake::sync::Mutex;
use mnemos_alloc::containers::{HeapArc, HeapArray, HeapFixedVec};
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// SerialMux is the registered driver type
pub struct SerialMuxService;

impl RegisteredDriver for SerialMuxService {
    type Request = Request;
    type Response = Response;
    type Error = SerialMuxError;
    const UUID: Uuid = crate::registry::known_uuids::kernel::SERIAL_MUX;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

pub enum Request {
    RegisterPort { port_id: u16, capacity: usize },
}

pub enum Response {
    PortRegistered(PortHandle),
}

#[derive(Debug, Eq, PartialEq)]
pub enum SerialMuxError {
    DuplicateItem,
    RegistryFull,
}

/// A `PortHandle` is the interface received after opening a virtual serial port
/// using a [`SerialMuxClient`].
pub struct PortHandle {
    port: u16,
    cons: bbq::Consumer,
    outgoing: bbq::MpscProducer,
    max_frame: usize,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// A `SerialMuxClient` is the client interface of the [`SerialMuxService`].
///
/// This client allows opening virtual serial ports, returning a [`PortHandle`]
/// representing the opened port.
pub struct SerialMuxClient {
    prod: KernelHandle<SerialMuxService>,
    reply: Reusable<Envelope<Result<Response, SerialMuxError>>>,
}

impl SerialMuxClient {
    /// Obtain a `SerialMuxClient`
    ///
    /// If the [`SerialMuxServer`] hasn't been registered yet, we will retry until it has been
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match SerialMuxClient::from_registry_no_retry(kernel).await {
                Some(port) => return port,
                None => {
                    // SerialMux probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain a `SerialMuxClient`
    ///
    /// Does NOT attempt to get a [`SerialMuxServer`] handle more than once.
    ///
    /// Prefer [`SerialMuxClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let prod = kernel
            .with_registry(|reg| reg.get::<SerialMuxService>())
            .await?;

        Some(SerialMuxClient {
            prod,
            reply: Reusable::new_async(kernel).await,
        })
    }

    pub async fn open_port(&mut self, port_id: u16, capacity: usize) -> Option<PortHandle> {
        let resp = self
            .prod
            .request_oneshot(Request::RegisterPort { port_id, capacity }, &self.reply)
            .await
            .ok()?;
        let body = resp.body.ok()?;

        let Response::PortRegistered(port) = body;
        Some(port)
    }
}

impl PortHandle {
    /// Helper method if you only need to open one port.
    ///
    /// Same as calling [SerialMuxClient::from_registry()] then immediately calling
    /// [SerialMuxClient::open_port()].
    ///
    /// If you need to open multiple ports at once, probably get a [SerialMuxClient] instead
    /// to reuse it for both ports
    pub async fn open(kernel: &'static Kernel, port_id: u16, capacity: usize) -> Option<Self> {
        let mut client = SerialMuxClient::from_registry(kernel).await;
        client.open_port(port_id, capacity).await
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn consumer(&self) -> &bbq::Consumer {
        &self.cons
    }

    pub async fn send(&self, data: &[u8]) {
        // This is lazy, and could probably be done with bigger chunks.
        let msg_chunk = self.max_frame / 2;

        for chunk in data.chunks(msg_chunk) {
            let enc_chunk = cobs::max_encoding_length(chunk.len() + 2);
            let mut wgr = self.outgoing.send_grant_exact(enc_chunk + 1).await;
            let mut encoder = cobs::CobsEncoder::new(&mut wgr);
            encoder.push(&self.port.to_le_bytes()).unwrap();
            encoder.push(chunk).unwrap();
            let used = encoder.finalize().unwrap();
            wgr[used] = 0;
            wgr.commit(used + 1);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

/// Server implementation for the [`SerialMuxService`].
pub struct SerialMuxServer;

impl SerialMuxServer {
    /// Register the `SerialMuxServer`.
    ///
    /// Will retry to obtain a [`SimpleSerialClient`] until success.
    pub async fn register(
        kernel: &'static Kernel,
        max_ports: usize,
        max_frame: usize,
    ) -> Result<(), RegistrationError> {
        loop {
            match SerialMuxServer::register_no_retry(kernel, max_ports, max_frame).await {
                Ok(_) => break,
                Err(RegistrationError::SerialPortNotFound) => {
                    // Uart probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
                Err(e) => {
                    panic!("uhhhh {e:?}");
                }
            }
        }
        Ok(())
    }

    /// Register the SerialMuxServer.
    ///
    /// Does NOT attempt to obtain a [`SimpleSerialClient`] more than once.
    ///
    /// Prefer [`SerialMuxServer::register`] unless you will not be spawning one around
    /// the same time as registering this server.
    pub async fn register_no_retry(
        kernel: &'static Kernel,
        max_ports: usize,
        max_frame: usize,
    ) -> Result<(), RegistrationError> {
        let mut serial_handle = SimpleSerialClient::from_registry(kernel)
            .await
            .ok_or(RegistrationError::SerialPortNotFound)?;
        let serial_port = serial_handle
            .get_port()
            .await
            .ok_or(RegistrationError::NoSerialPortAvailable)?;

        let (sprod, scons) = serial_port.split();
        let sprod = sprod.into_mpmc_producer().await;

        let ports = kernel.heap().allocate_fixed_vec(max_ports).await;
        let imutex = kernel
            .heap()
            .allocate_arc(Mutex::new(MuxingInfo {
                ports,
                kernel,
                max_frame,
            }))
            .await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, max_ports).await.split();
        let buf = kernel.heap().allocate_array_with(|| 0, max_frame).await;
        let commander = CommanderTask {
            cmd: cmd_cons,
            out: sprod,
            mux: imutex.clone(),
        };
        let muxer = IncomingMuxerTask {
            incoming: scons,
            mux: imutex,
            buf,
            idx: 0,
        };

        kernel.spawn(commander.run()).await;

        kernel
            .spawn(async move {
                muxer.run().await;
            })
            .await;

        kernel
            .with_registry(|reg| reg.register_konly::<SerialMuxService>(&cmd_prod))
            .await
            .map_err(|_| RegistrationError::MuxAlreadyRegistered)?;

        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum RegistrationError {
    SerialPortNotFound,
    NoSerialPortAvailable,
    MuxAlreadyRegistered,
}

struct PortInfo {
    port: u16,
    upstream: bbq::SpscProducer,
}

struct MuxingInfo {
    kernel: &'static Kernel,
    ports: HeapFixedVec<PortInfo>,
    max_frame: usize,
}

struct CommanderTask {
    cmd: KConsumer<Message<SerialMuxService>>,
    out: bbq::MpscProducer,
    mux: HeapArc<Mutex<MuxingInfo>>,
}

struct IncomingMuxerTask {
    buf: HeapArray<u8>,
    idx: usize,
    incoming: bbq::Consumer,
    mux: HeapArc<Mutex<MuxingInfo>>,
}

impl MuxingInfo {
    async fn register_port(
        &mut self,
        port_id: u16,
        capacity: usize,
        outgoing: &bbq::MpscProducer,
    ) -> Result<PortHandle, SerialMuxError> {
        if self.ports.is_full() {
            return Err(SerialMuxError::RegistryFull);
        }
        if self.ports.iter().any(|p| p.port == port_id) {
            return Err(SerialMuxError::DuplicateItem);
        }
        let (prod, cons) = bbq::new_spsc_channel(self.kernel.heap(), capacity).await;

        self.ports
            .push(PortInfo {
                port: port_id,
                upstream: prod,
            })
            .map_err(|_| SerialMuxError::RegistryFull)?;

        let ph = PortHandle {
            port: port_id,
            cons,
            outgoing: outgoing.clone(),
            max_frame: self.max_frame,
        };

        Ok(ph)
    }
}

// impl CommanderTask

impl CommanderTask {
    async fn run(self) {
        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let Message { msg: req, reply } = msg;
            match req.body {
                Request::RegisterPort { port_id, capacity } => {
                    let res = {
                        let mut mux = self.mux.lock().await;
                        mux.register_port(port_id, capacity, &self.out).await
                    }
                    .map(Response::PortRegistered);

                    let resp = req.reply_with(res);

                    reply.reply_konly(resp).await.map_err(drop).unwrap();
                }
            }
        }
    }
}

// impl IncomingMuxerTask

impl IncomingMuxerTask {
    async fn run(mut self) {
        loop {
            let mut rgr = self.incoming.read_grant().await;
            let mut used = 0;
            for ch in rgr.split_inclusive_mut(|&num| num == 0) {
                used += ch.len();

                if ch.last() != Some(&0) {
                    // This is the last chunk, and it doesn't end with a zero.
                    // just add it to the accumulator, if we can.
                    if (self.idx + ch.len()) <= self.buf.len() {
                        self.buf[self.idx..][..ch.len()].copy_from_slice(ch);
                        self.idx += ch.len();
                    } else {
                        warn!("Overfilled accumulator");
                        self.idx = 0;
                    }

                    // Either we overfilled, or this was the last data. Move on.
                    continue;
                }

                // Okay, we know that we have a zero terminated item. Do we have anything residual?
                let buf = if self.idx == 0 {
                    // Yes, no pending data, just use the current chunk
                    ch
                } else {
                    // We have residual data, we need to copy the chunk to the end of the buffer
                    if (self.idx + ch.len()) <= self.buf.len() {
                        self.buf[self.idx..][..ch.len()].copy_from_slice(ch);
                        self.idx += ch.len();
                    } else {
                        warn!("Overfilled accumulator");
                        self.idx = 0;
                        continue;
                    }
                    &mut self.buf[..self.idx]
                };

                // Great! Now decode the cobs message in place.
                let used = match cobs::decode_in_place(buf) {
                    Ok(u) if u < 3 => {
                        warn!("Cobs decode too short!");
                        continue;
                    }
                    Ok(u) => u,
                    Err(_) => {
                        warn!("Cobs decode failed!");
                        continue;
                    }
                };

                let mut port = [0u8; 2];
                let (portb, datab) = buf[..used].split_at(2);
                port.copy_from_slice(portb);
                let port_id = u16::from_le_bytes(port);

                // Great, now we have a message! Let's see if we have someone listening to this port
                let mux = self.mux.lock().await;
                if let Some(port) = mux.ports.iter().find(|p| p.port == port_id) {
                    if let Some(mut wgr) = port.upstream.send_grant_exact_sync(datab.len()) {
                        wgr.copy_from_slice(datab);
                        wgr.commit(datab.len());
                        debug!(port_id, len = datab.len(), "Sent bytes to port");
                    } else {
                        warn!(port_id, len = datab.len(), "Discarded bytes, full buffer");
                    }
                } else {
                    warn!(port_id, len = datab.len(), "Discarded bytes, no consumer");
                }
            }
            rgr.release(used);
            debug!(used, "processed incoming bytes");
        }
    }
}
