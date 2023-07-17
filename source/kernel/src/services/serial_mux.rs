//! # Serial Multiplexor
//!
//! Allows the creation of virtual "ports" over a single serial link
//!
//! This module includes the service definition, client definition, as well
//! as a server definition that relies on the [`SimpleSerial`][crate::services::simple_serial]
//! service to provide the service implementation.

use core::time::Duration;

use crate::comms::bbq::GrantR;
use crate::tracing::{self, debug, warn, Level};
use crate::{
    comms::{
        bbq,
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    registry::{Envelope, KernelHandle, Message, RegisteredDriver},
    services::simple_serial::SimpleSerialClient,
    Kernel,
};
use maitake::sync::Mutex;
use mnemos_alloc::containers::{Arc, FixedVec};
use sermux_proto::PortChunk;
use uuid::Uuid;

// Well known ports live in the sermux_proto crate
pub use sermux_proto::WellKnown;

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
            reply: Reusable::new_async().await,
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
            let pc = PortChunk::new(self.port, chunk);
            let needed = pc.buffer_required();
            let mut wgr = self.outgoing.send_grant_exact(needed).await;
            let used = pc
                .encode_to(&mut wgr)
                .expect("sermux encoding should not fail")
                .len();
            wgr.commit(used);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

/// Server implementation for the [`SerialMuxService`].
pub struct SerialMuxServer;

#[derive(Copy, Clone, Debug)]
pub struct SerialMuxSettings {
    max_ports: u16,
    max_frame: usize,
}

impl SerialMuxServer {
    /// Register the `SerialMuxServer`.
    ///
    /// Registering a `SerialMuxServer` will always acquire a
    /// [`SimpleSerialClient`] to access the serial port.
    ///
    /// Will retry to obtain a [`SimpleSerialClient`] until success.
    #[tracing::instrument(
        name = "KeyboardMuxServer::register",
        level = Level::DEBUG,
        skip(kernel),
        err(Debug),
    )]
    pub async fn register(
        kernel: &'static Kernel,
        settings: SerialMuxSettings,
    ) -> Result<(), RegistrationError> {
        loop {
            match SerialMuxServer::register_no_retry(kernel, settings).await {
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
    /// Registering a `SerialMuxServer` will always acquire a
    /// [`SimpleSerialClient`] to access the serial port.
    ///
    /// This method does NOT attempt to obtain a [`SimpleSerialClient`] more
    /// than once. Prefer [`SerialMuxServer::register`] unless you will not be
    /// spawning one around the same time as registering this server.
    pub async fn register_no_retry(
        kernel: &'static Kernel,
        SerialMuxSettings {
            max_ports,
            max_frame,
        }: SerialMuxSettings,
    ) -> Result<(), RegistrationError> {
        let max_ports = max_ports as usize;
        let mut serial_handle = SimpleSerialClient::from_registry(kernel)
            .await
            .ok_or(RegistrationError::SerialPortNotFound)?;
        let serial_port = serial_handle
            .get_port()
            .await
            .ok_or(RegistrationError::NoSerialPortAvailable)?;

        let (sprod, scons) = serial_port.split();
        let sprod = sprod.into_mpmc_producer().await;

        let ports = FixedVec::new(max_ports).await;
        let imutex = Arc::new(Mutex::new(MuxingInfo { ports, max_frame })).await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(max_ports).await.split();
        let buf = FixedVec::new(max_frame).await;
        let commander = CommanderTask {
            cmd: cmd_cons,
            out: sprod,
            mux: imutex.clone(),
        };
        let muxer = IncomingMuxerTask {
            incoming: scons,
            mux: imutex,
            buf,
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

impl SerialMuxSettings {
    pub const DEFAULT_MAX_PORTS: u16 = 16;
    pub const DEFAULT_MAX_FRAME: usize = 512;

    pub fn with_max_ports(self, max_ports: u16) -> Self {
        Self { max_ports, ..self }
    }

    pub fn with_max_frame(self, max_frame: usize) -> Self {
        Self { max_frame, ..self }
    }
}

impl Default for SerialMuxSettings {
    fn default() -> Self {
        Self {
            max_ports: Self::DEFAULT_MAX_PORTS,
            max_frame: Self::DEFAULT_MAX_FRAME,
        }
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
    ports: FixedVec<PortInfo>,
    max_frame: usize,
}

struct CommanderTask {
    cmd: KConsumer<Message<SerialMuxService>>,
    out: bbq::MpscProducer,
    mux: Arc<Mutex<MuxingInfo>>,
}

struct IncomingMuxerTask {
    buf: FixedVec<u8>,
    incoming: bbq::Consumer,
    mux: Arc<Mutex<MuxingInfo>>,
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
        if self.ports.as_slice().iter().any(|p| p.port == port_id) {
            return Err(SerialMuxError::DuplicateItem);
        }
        let (prod, cons) = bbq::new_spsc_channel(capacity).await;

        self.ports
            .try_push(PortInfo {
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
            let rgr = self.incoming.read_grant().await;

            // No data, no worries
            if !take_from_grant(&mut self.buf, rgr) {
                continue;
            }

            //////////////////////////////////////////////////////////////////
            // No early returns/continues until the jerb is done unless you
            // clear the buffer!
            //
            let (port_id, datab) = match try_decode(self.buf.as_slice_mut()) {
                Some(a) => a,
                None => {
                    // Nothing decoded, which means decoding has failed.
                    self.buf.clear();
                    continue;
                }
            };

            // Great, now we have a message! Let's see if we have someone listening to this port
            let mux = self.mux.lock().await;
            if let Some(port) = mux.ports.as_slice().iter().find(|p| p.port == port_id) {
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

            // Now we clear the buffer
            self.buf.clear();
            //
            // jerb done!
            //////////////////////////////////////////////////////////////////
        }
    }
}

/// Takes data from the grant
///
/// Returns true if the buffer is now ready for decoding
/// Returns false if more data is needed
///
/// If the grant has been overfilled, the buffer will be cleared.
fn take_from_grant(buffer: &mut FixedVec<u8>, grant: GrantR) -> bool {
    let mut try_decode = false;

    // How many bytes should we try to take?
    let to_use = match grant.iter().position(|&v| v == 0) {
        Some(idx) => {
            try_decode = true;
            &grant[..idx + 1]
        }
        None => &grant,
    };

    // Okay, add those to the buffer
    if buffer.try_extend_from_slice(to_use).is_err() {
        warn!("Overfilled accumulator");
        buffer.clear();
        try_decode = false;
    }

    // Now we can release the grant
    let used = to_use.len();
    grant.release(used);
    debug!(used, "consumed incoming bytes");

    try_decode
}

/// Tries to decode a port and message from the given buffer
///
/// Either way, you should probably clear the buffer when you are done.
fn try_decode<'a>(buffer: &'a mut [u8]) -> Option<(u16, &'a [u8])> {
    let used = match cobs::decode_in_place(buffer) {
        Ok(u) if u < 3 => {
            warn!("Cobs decode too short!");
            return None;
        }
        Ok(u) => u,
        Err(_) => {
            warn!("Cobs decode failed!");
            return None;
        }
    };

    let total = buffer.get(..used)?;

    let mut port = [0u8; 2];
    let (portb, datab) = total.split_at(2);
    port.copy_from_slice(portb);
    let port_id = u16::from_le_bytes(port);

    Some((port_id, datab))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::comms::bbq::{Consumer, SpscProducer};
    use core::ops::Deref;

    struct Stuff {
        prod: SpscProducer,
        cons: Consumer,
        buffer: FixedVec<u8>,
    }

    impl Stuff {
        fn setup() -> Self {
            let (prod, cons) =
                futures::executor::block_on(async { bbq::new_spsc_channel(128).await });
            let buffer = futures::executor::block_on(async { FixedVec::<u8>::new(64).await });
            Stuff { prod, cons, buffer }
        }

        fn send(&self, data: &[u8]) {
            let mut wgr = self.prod.send_grant_exact_sync(data.len()).unwrap();
            wgr.copy_from_slice(data);
            wgr.commit(data.len());
        }

        fn read(&self) -> GrantR {
            self.cons.read_grant_sync().unwrap()
        }

        fn clear(&mut self) {
            self.buffer.clear();
        }
    }

    /// Make sure we can decode messages
    #[test]
    fn simple_decode() {
        const MESSAGE: &[u8] = &[0x01, 0x01, 0x02, b'!', 0x00];
        let mut ctxt = Stuff::setup();
        ctxt.send(MESSAGE);

        let rgr = ctxt.read();

        assert!(take_from_grant(&mut ctxt.buffer, rgr));
        assert_eq!(ctxt.buffer.as_slice(), MESSAGE);
        let (port_id, data) = try_decode(ctxt.buffer.as_slice_mut()).unwrap();
        assert_eq!(port_id, 0);
        assert_eq!(data, b"!");
    }

    /// Make sure we successfully report empty messages as failed
    #[test]
    fn empty_message() {
        const MESSAGE: &[u8] = &[0x01, 0x01, 0x01, 0x00];
        let mut ctxt = Stuff::setup();
        ctxt.send(MESSAGE);

        let rgr = ctxt.read();

        assert!(take_from_grant(&mut ctxt.buffer, rgr));
        assert_eq!(ctxt.buffer.as_slice(), MESSAGE);
        assert!(try_decode(ctxt.buffer.as_slice_mut()).is_none());
    }

    /// OVERfill the buffer, ensure we recover
    #[test]
    fn fillup() {
        const MESSAGE_GOOD: &[u8] = &[0x01, 0x01, 0x02, b'!', 0x00];
        const MESSAGE_BAD: &[u8] = &[0x01, 0x01, 0x02, b'!'];
        let mut ctxt = Stuff::setup();

        let times = ctxt.buffer.capacity() / MESSAGE_BAD.len();

        // ALMOST fill up the buffer
        for _ in 0..times {
            ctxt.send(MESSAGE_BAD);
            let rgr = ctxt.read();
            assert!(!take_from_grant(&mut ctxt.buffer, rgr));
            assert!(!ctxt.buffer.is_empty());
        }

        // oops overflow
        ctxt.send(MESSAGE_BAD);
        let rgr = ctxt.read();
        assert!(!take_from_grant(&mut ctxt.buffer, rgr));
        assert!(ctxt.buffer.is_empty());

        // Good messages still work after recovery
        ctxt.send(MESSAGE_GOOD);

        let rgr = ctxt.read();

        assert!(take_from_grant(&mut ctxt.buffer, rgr));
        assert_eq!(ctxt.buffer.as_slice(), MESSAGE_GOOD);
        let (port_id, data) = try_decode(ctxt.buffer.as_slice_mut()).unwrap();
        assert_eq!(port_id, 0);
        assert_eq!(data, b"!");
    }

    /// We only consume up to one message at a time
    #[test]
    fn partial_take() {
        const MESSAGE: &[u8] = &[0x01, 0x01, 0x02, b'!', 0x00];
        let mut ctxt = Stuff::setup();
        ctxt.send(MESSAGE);
        ctxt.send(MESSAGE);

        let rgr = ctxt.read();

        assert!(take_from_grant(&mut ctxt.buffer, rgr));
        assert_eq!(ctxt.buffer.as_slice(), MESSAGE);
        let (port_id, data) = try_decode(ctxt.buffer.as_slice_mut()).unwrap();
        assert_eq!(port_id, 0);
        assert_eq!(data, b"!");
        ctxt.clear();

        let rgr = ctxt.read();
        assert_eq!(rgr.deref(), MESSAGE);

        assert!(take_from_grant(&mut ctxt.buffer, rgr));
        assert_eq!(ctxt.buffer.as_slice(), MESSAGE);
        let (port_id, data) = try_decode(ctxt.buffer.as_slice_mut()).unwrap();
        assert_eq!(port_id, 0);
        assert_eq!(data, b"!");
        ctxt.clear();
    }
}
