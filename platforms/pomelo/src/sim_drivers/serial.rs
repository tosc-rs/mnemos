use std::{net::SocketAddr, pin::pin, sync::Arc};

use futures::{
    channel::mpsc::{Receiver, Sender},
    select,
};
use futures_util::{FutureExt, Stream, StreamExt};
use mnemos_kernel::{
    comms::{
        bbq::{new_bidi_channel, BidiHandle},
        kchannel::KChannel,
    },
    registry::Message,
    services::simple_serial::{Request, Response, SimpleSerialError, SimpleSerialService},
    Kernel,
};
use tracing::{info, info_span, trace, warn, Instrument};

pub struct SerialRequest {
    port: usize,
    handle: BidiHandle,
}

use async_std::{sync::Condvar, task::spawn_local};
pub struct Serial {}

impl Serial {
    pub async fn register(
        kernel: &'static Kernel,
        incoming_size: usize,
        outgoing_size: usize,
        port: u16,
        irq: Arc<Condvar>,
        recv: Receiver<u8>,
    ) -> Result<(), ()> {
        trace!("register::enter");
        let (a_ring, b_ring) = new_bidi_channel(incoming_size, outgoing_size).await;
        let (prod, cons) = KChannel::<Message<SimpleSerialService>>::new_async(2)
            .await
            .split();

        trace!("spawning port request");
        kernel
            .spawn(async move {
                let handle = b_ring;

                // Reply to the first request, giving away the serial port
                let req = cons.dequeue_async().await.map_err(drop).unwrap();
                let Request::GetPort = req.msg.body;
                let resp = req.msg.reply_with(Ok(Response::PortHandle { handle }));

                req.reply.reply_konly(resp).await.map_err(drop).unwrap();

                // And deny all further requests after the first
                loop {
                    let req = cons.dequeue_async().await.map_err(drop).unwrap();
                    let Request::GetPort = req.msg.body;
                    let resp = req
                        .msg
                        .reply_with(Err(SimpleSerialError::AlreadyAssignedPort));
                    req.reply.reply_konly(resp).await.map_err(drop).unwrap();
                }
            })
            .await;

        spawn_local(
            async move {
                let mut handle = a_ring;
                process_stream(&mut handle, recv, irq.clone())
                    .instrument(info_span!("process_stream", ?port))
                    .await
            }
            .instrument(info_span!("Serial", ?port)),
        );
        kernel
            .with_registry(|reg| reg.register_konly::<SimpleSerialService>(&prod))
            .await
            .map_err(drop)
    }
}

async fn process_stream(
    handle: &mut BidiHandle,
    mut in_stream: impl Stream<Item = u8>,
    irq: Arc<Condvar>,
) {
    info!("processing serial stream");
    // Wait until either the socket has data to read, or the other end of
    // the BBQueue has data to write.
    let in_stream = pin!(in_stream);
    let mut in_stream = in_stream.fuse();
    loop {
        select! {
            // The kernel wants to write something.
            outmsg = handle.consumer().read_grant().fuse() => {
                info!(len = outmsg.len(), "Got outgoing message");
                // let wall = stream.write_all(&outmsg);
                // wall.await.unwrap();
                let len = outmsg.len();
                outmsg.release(len);
            },
            inmsg = in_stream.next() => {
                if let Some(inmsg) = inmsg {
                // Simulate an "interrupt", waking the kernel if it's waiting
                // an IRQ.
                irq.notify_one();
                // TODO we can do better than single bytes
                let used = 1;
                let mut in_grant = handle.producer().send_grant_max(used).await;
                in_grant[0] = inmsg;
                info!(len = used, "Got incoming message",);
                in_grant.commit(used);
                irq.notify_one();
                }

            }
        }
    }
}
