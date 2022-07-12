use mnemos_alloc::containers::{HeapFixedVec, HeapArc};

use crate::{comms::{
    bbq,
    kchannel::{KChannel, KConsumer, KProducer},
}, Kernel};
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
    RegisterPort {
        port_id: u16,
        capacity: usize,
    },
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
                    resp.enqueue_async(res.map(|ph| Response::PortRegistered(ph))).await.map_err(drop).unwrap();
                },
            }
        }
    }
}

pub struct SerialMux {
    kernel: &'static Kernel,
    ports: HeapFixedVec<PortInfo>,
}

impl SerialMux {
    async fn register_port(&mut self, port_id: u16, capacity: usize, outgoing: &bbq::MpscProducer) -> Result<PortHandle, ()> {
        if self.ports.is_full() || self.ports.iter().any(|p| p.port == port_id) {
            return Err(());
        }
        let (prod, cons) = bbq::new_spsc_channel(self.kernel.heap(), capacity).await;

        self.ports.push(PortInfo { port: port_id, upstream: prod }).map_err(drop)?;

        let ph = PortHandle {
            port: port_id,
            cons,
            outgoing: outgoing.clone(),
        };

        Ok(ph)
    }
}

struct SerialMuxer {
    incoming: bbq::Consumer,
    mux: HeapArc<Mutex<SerialMux>>,
}

impl SerialMuxer {
    async fn run(self) {
        todo!()
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
        serial_port: bbq::BidiHandle,
    ) -> KProducer<Message> {
        let (sprod, scons) = serial_port.split();
        let sprod = sprod.into_mpmc_producer().await;

        let ports = kernel.heap().allocate_fixed_vec(max_ports).await;
        let imutex = kernel.heap().allocate_arc(Mutex::new(SerialMux { ports, kernel })).await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, max_ports).await.split();
        let commander = Commander {
            cmd: cmd_cons,
            out: sprod,
            mux: imutex.clone(),
        };
        let muxer = SerialMuxer { incoming: scons, mux: imutex };

        kernel.spawn(async move {
            commander.run().await;
        }).await;

        kernel.spawn(async move {
            muxer.run().await;
        }).await;

        cmd_prod
    }
}
