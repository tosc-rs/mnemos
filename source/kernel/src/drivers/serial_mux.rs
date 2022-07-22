use crate::{
    comms::{
        bbq,
        kchannel::{KChannel, KConsumer},
        rosc::Rosc,
    },
    registry::{
        simple_serial::SimpleSerial, HMessage, KernelHandle, Message, RegisteredDriver, ReplyTo,
    },
    Kernel,
};
use maitake::sync::Mutex;
use mnemos_alloc::containers::{HeapArc, HeapArray, HeapFixedVec};
use tracing::{debug, warn};
use uuid::{uuid, Uuid};

/// SerialMux is the registered driver type
pub struct SerialMux {
    _inner: (),
}

/// A PortHandle is the interface received after opening a virtual serial port
pub struct PortHandle {
    port: u16,
    cons: bbq::Consumer,
    outgoing: bbq::MpscProducer,
    max_frame: usize,
}

/// A SerialMuxHandle is the client interface of the [SerialMux].
pub struct SerialMuxHandle {
    prod: KernelHandle<SerialMux>,
    reply: Rosc<HMessage<Result<Response, ()>>>,
}

pub enum Request {
    RegisterPort { port_id: u16, capacity: usize },
}

pub enum Response {
    PortRegistered(PortHandle),
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
    cmd: KConsumer<Message<SerialMux>>,
    out: bbq::MpscProducer,
    mux: HeapArc<Mutex<MuxingInfo>>,
}

struct IncomingMuxerTask {
    buf: HeapArray<u8>,
    idx: usize,
    incoming: bbq::Consumer,
    mux: HeapArc<Mutex<MuxingInfo>>,
}

// impl SerialMux

impl RegisteredDriver for SerialMux {
    type Request = Request;
    type Response = Response;
    const UUID: Uuid = uuid!("54c983fa-736f-4223-b90d-c4360a308647");
}

impl SerialMux {
    pub async fn register(
        kernel: &'static Kernel,
        max_ports: usize,
        max_frame: usize,
    ) -> Result<(), ()> {
        let serial_handle = SimpleSerial::from_registry(kernel).await.ok_or(())?;
        let serial_port = serial_handle.get_port().await.ok_or(())?;

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

        kernel
            .spawn(async move {
                commander.run().await;
            })
            .await;

        kernel
            .spawn(async move {
                muxer.run().await;
            })
            .await;

        kernel
            .with_registry(|reg| reg.set_konly::<SerialMux>(&cmd_prod))
            .await
            .expect("Only registered once");

        Ok(())
    }
}

// impl PortHandle

impl PortHandle {
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

// impl SerialMuxHandle

impl SerialMuxHandle {
    pub async fn from_registry(kernel: &'static Kernel) -> Option<Self> {
        let prod = kernel.with_registry(|reg| reg.get::<SerialMux>()).await?;

        Some(SerialMuxHandle {
            prod,
            reply: Rosc::new_async(kernel).await,
        })
    }

    pub async fn open_port(&self, port_id: u16, capacity: usize) -> Option<PortHandle> {
        self.prod
            .send(
                Request::RegisterPort { port_id, capacity },
                ReplyTo::Rosc(self.reply.sender().ok()?),
            )
            .await
            .ok()?;

        let resp = self.reply.receive().await.ok()?;
        let body = resp.body.ok()?;

        let Response::PortRegistered(port) = body;
        Some(port)
    }
}

// impl MuxingInfo

impl MuxingInfo {
    async fn register_port(
        &mut self,
        port_id: u16,
        capacity: usize,
        outgoing: &bbq::MpscProducer,
    ) -> Result<PortHandle, ()> {
        if self.ports.is_full() || self.ports.iter().any(|p| p.port == port_id) {
            return Err(());
        }
        let (prod, cons) = bbq::new_spsc_channel(self.kernel.heap(), capacity).await;

        self.ports
            .push(PortInfo {
                port: port_id,
                upstream: prod,
            })
            .map_err(drop)?;

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
            let msg = self.cmd.dequeue_async().await.unwrap();
            let Message { msg: req, reply } = msg;
            match req {
                HMessage {
                    body: Request::RegisterPort { port_id, capacity },
                } => {
                    let res = {
                        let mut mux = self.mux.lock().await;
                        mux.register_port(port_id, capacity, &self.out).await
                    }
                    .map(Response::PortRegistered);
                    reply.reply_konly(res).await.unwrap();
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
