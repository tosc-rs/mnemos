use mnemos_alloc::containers::{HeapArc, HeapArray, HeapFixedVec};
use tracing::{debug, warn};

use crate::{
    comms::{
        bbq,
        kchannel::{KChannel, KConsumer, KProducer},
    },
    Kernel,
};
use maitake::sync::Mutex;

struct PortInfo {
    pub port: u16,
    pub upstream: bbq::SpscProducer,
}

pub struct Message {
    pub req: Request,
    pub resp: KProducer<Result<Response, ()>>,
}

pub enum Request {
    RegisterPort { port_id: u16, capacity: usize },
}

pub enum Response {
    PortRegistered(PortHandle),
}

struct Commander {
    cmd: KConsumer<Message>,
    out: bbq::MpscProducer,
    mux: HeapArc<Mutex<SerialMux>>,
}

impl Commander {
    async fn run(self) {
        loop {
            let msg = self.cmd.dequeue_async().await.unwrap();
            let Message { req, resp } = msg;
            match req {
                Request::RegisterPort { port_id, capacity } => {
                    let res = {
                        let mut mux = self.mux.lock().await;
                        mux.register_port(port_id, capacity, &self.out).await
                    };
                    resp.enqueue_async(res.map(|ph| Response::PortRegistered(ph)))
                        .await
                        .map_err(drop)
                        .unwrap();
                }
            }
        }
    }
}

pub struct SerialMux {
    kernel: &'static Kernel,
    ports: HeapFixedVec<PortInfo>,
}

impl SerialMux {
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
        };

        Ok(ph)
    }
}

struct SerialMuxer {
    buf: HeapArray<u8>,
    idx: usize,
    incoming: bbq::Consumer,
    mux: HeapArc<Mutex<SerialMux>>,
}

impl SerialMuxer {
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

pub struct PortHandle {
    port: u16,
    cons: bbq::Consumer,
    outgoing: bbq::MpscProducer,
}

impl PortHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn consumer(&self) -> &bbq::Consumer {
        &self.cons
    }

    pub fn producer(&self) -> &bbq::MpscProducer {
        &self.outgoing
    }
}

impl SerialMux {
    pub async fn new(
        kernel: &'static Kernel,
        max_ports: usize,
        max_frame: usize,
        serial_port: bbq::BidiHandle,
    ) -> KProducer<Message> {
        let (sprod, scons) = serial_port.split();
        let sprod = sprod.into_mpmc_producer().await;

        let ports = kernel.heap().allocate_fixed_vec(max_ports).await;
        let imutex = kernel
            .heap()
            .allocate_arc(Mutex::new(SerialMux { ports, kernel }))
            .await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, max_ports).await.split();
        let buf = kernel.heap().allocate_array_with(|| 0, max_frame).await;
        let commander = Commander {
            cmd: cmd_cons,
            out: sprod,
            mux: imutex.clone(),
        };
        let muxer = SerialMuxer {
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

        cmd_prod
    }
}
