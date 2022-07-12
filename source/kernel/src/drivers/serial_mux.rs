use core::ops::{Deref, DerefMut};

use mnemos_alloc::containers::{HeapFixedVec, HeapArc};

use crate::{comms::{
    bbq::bidi::{BBQBidiHandle, GrantW, GrantR, new_bidi_channel},
    kchannel::{KChannel, KConsumer, KProducer},
}, Kernel};
use maitake::sync::Mutex;


struct PortInfo {
    pub port: u16,
    pub upstream: BidiProducer,
}

pub struct Message {
    pub req: Request,
    pub resp: KProducer<Result<Response, ()>>,
}

pub enum Request {
    RegisterPort {
        port_id: u16,
        capacity_in: usize,
        capacity_out: usize,
    },
}

pub enum Response {
    PortRegistered(PortHandle),
}

struct Outgoing {
    producer: BidiProducer,
}

struct Commander {
    cmd: KConsumer<Message>,
    out: HeapArc<Mutex<Outgoing>>,
    mux: HeapArc<Mutex<SerialMux>>,
}

impl Commander {
    async fn run(self) {
        loop {
            let msg = self.cmd.dequeue_async().await.unwrap();
            let Message { req, resp } = msg;
            match req {
                Request::RegisterPort { port_id, capacity_in, capacity_out } => {
                    let res = {
                        let mut mux = self.mux.lock().await;
                        mux.register_port(port_id, capacity_in, capacity_out).await
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
    async fn register_port(&mut self, port_id: u16, capacity_in: usize, capacity_out: usize) -> Result<PortHandle, ()> {
        if self.ports.is_full() || self.ports.iter().any(|p| p.port == port_id) {
            return Err(());
        }


        todo!()
    }
}

struct SerialMuxer {
    incoming: BidiConsumer,
    mux: HeapArc<Mutex<SerialMux>>,
}

impl SerialMuxer {
    async fn run(self) {
        todo!()
    }
}

pub struct PortHandle {
    port: u16,
    cons: BidiConsumer,
    outgoing: HeapArc<Mutex<Outgoing>>,
}

enum Step {
    Cmd(Message),
    Inc(GrantR),
    Out(u16),
}

impl SerialMux {
    pub async fn new(
        kernel: &'static Kernel,
        max_ports: usize,
        serial_port: BBQBidiHandle,
    ) -> KProducer<Message> {
        let (sprod, scons) = serial_port.split();

        let omutex = kernel.heap().allocate_arc(Mutex::new(Outgoing { producer: sprod })).await;
        let ports = kernel.heap().allocate_fixed_vec(max_ports).await;
        let imutex = kernel.heap().allocate_arc(Mutex::new(SerialMux { ports })).await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, max_ports).await.split();
        let commander = Commander {
            cmd: cmd_cons,
            out: omutex,
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
