use core::ops::{Deref, DerefMut};

use mnemos_alloc::containers::HeapFixedVec;

use crate::{comms::{
    bbq::{BBQBidiHandle, new_bidi_channel, GrantW, GrantR},
    kchannel::{KChannel, KConsumer, KProducer},
}, Kernel};
use maitake::sync::Mutex;

use futures::{select_biased, FutureExt, pin_mut};

pub struct PortInfo {
    pub port: u16,
    pub upstream: BBQBidiHandle,
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

pub struct SerialMux {
    serial_port: BBQBidiHandle,
    notifications: KConsumer<u16>,
    ports: HeapFixedVec<PortInfo>,
    cmd: KConsumer<Message>,
}

pub struct PortHandle {
    port: u16,
    notifier: KProducer<u16>,
    // TODO: I don't want this to be *exactly* a BBQBidiHandle, but it's pretty close.
    // We just won't use some of the wait cells, but that's a memory optimization for later.
    stream: BBQBidiHandle,
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
        let cons: KConsumer<u16> = KChannel::new_async(kernel, max_ports).await.into_consumer();
        let ports: HeapFixedVec<PortInfo> = kernel.heap().allocate_fixed_vec(max_ports).await;
        let (cmd_prod, cmd_cons) = KChannel::<Message>::new_async(kernel, 4).await.split();

        let mux = Self {
            serial_port,
            notifications: cons,
            ports,
            cmd: cmd_cons
        };

        kernel.spawn(async move {
            let mut mux = mux;
            mux.run().await;
        }).await;

        cmd_prod
    }

    pub async fn register_port(
        &mut self,
        kernel: &'static Kernel,
        port_id: u16,
        capacity_in: usize,
        capacity_out: usize,
    ) -> Result<PortHandle, ()> {
        if self.ports.is_full() || self.ports.iter().any(|p| p.port == port_id) {
            return Err(());
        }

        let (incoming_side, outgoing_side) = new_bidi_channel(kernel.heap(), capacity_in, capacity_out).await;

        // The handle WE hold can PUSH to the incoming side, and can PULL from the outgoing side
        self.ports.push(PortInfo { port: port_id, upstream: incoming_side }).map_err(drop)?;

        // The PortHandle can PUSH into the outgoing_side, and can PULL from the incoming side
        Ok(PortHandle {
            port: port_id,
            notifier: self.notifications.producer(),
            stream: outgoing_side,
        })
    }

    pub async fn run(&mut self) {
        loop {
            let next = {
                let cmds = self.cmd.dequeue_async().fuse();
                let inc_ser = self.serial_port.read_grant().fuse();
                let outgoing_notif = self.notifications.dequeue_async().fuse();
                pin_mut!(cmds);
                pin_mut!(inc_ser);
                pin_mut!(outgoing_notif);

                select_biased! {
                    cmd = cmds => {
                        match cmd {
                            Ok(cmd) => Step::Cmd(cmd),
                            Err(_) => continue,
                        }
                    }
                    inc = inc_ser => Step::Inc(inc),
                    out = outgoing_notif => {
                        match out {
                            Ok(out) => Step::Out(out),
                            Err(_) => continue,
                        }
                    },
                }
            };

            match next {
                Step::Cmd(cmd) => self.handle_cmd(cmd).await,
                Step::Inc(rgr) => self.handle_incoming(rgr).await,
                Step::Out(out) => self.handle_outgoing(out).await,
            }
        }
    }

    pub async fn handle_cmd(&mut self, cmd: Message) {

    }

    pub async fn handle_incoming(&mut self, rgr: GrantR) {

    }

    pub async fn handle_outgoing(&mut self, port: u16) {

    }
}

pub struct MuxGrantW {
    grant: GrantW,
    port: u16,
    notifier: KProducer<u16>,
}

impl MuxGrantW {
    pub async fn commit(self, used: usize) {
        self.grant.commit(used);
        if used > 0 {
            self.notifier.enqueue_async(self.port).await.ok();
        }
    }
}

impl Deref for MuxGrantW {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.grant.deref()
    }
}

impl DerefMut for MuxGrantW {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.grant.deref_mut()
    }
}

// async methods
impl PortHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn send_grant_max(&self, max: usize) -> MuxGrantW {
        let grant = self.stream.send_grant_max(max).await;
        MuxGrantW {
            grant,
            port: self.port,
            notifier: self.notifier.clone(),
        }
    }

    pub async fn send_grant_exact(&self, size: usize) -> MuxGrantW {
        let grant = self.stream.send_grant_exact(size).await;
        MuxGrantW {
            grant,
            port: self.port,
            notifier: self.notifier.clone(),
        }
    }

    pub async fn read_grant(&self) -> GrantR {
        self.stream.read_grant().await
    }
}
