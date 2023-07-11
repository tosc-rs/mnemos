use std::{net::SocketAddr, pin::pin, str::from_utf8, sync::Arc};

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
use sermux_proto::{PortChunk, WellKnown};
use tracing::{debug, error, info, info_span, trace, warn, Instrument};

pub struct SerialRequest {
    port: usize,
    handle: BidiHandle,
}

use async_std::{sync::Condvar, task::spawn_local};

use crate::term_iface::ECHO_TX;
pub struct Serial {}

impl Serial {
    pub async fn register(
        kernel: &'static Kernel,
        incoming_size: usize,
        outgoing_size: usize,
        port: u16,
        irq: Arc<Condvar>,
        recv: Receiver<u8>,
        recv_callback: fn(String),
    ) -> Result<(), ()> {
        let (a_ring, b_ring) = new_bidi_channel(incoming_size, outgoing_size).await;
        let (prod, cons) = KChannel::<Message<SimpleSerialService>>::new_async(2)
            .await
            .split();

        kernel
            .spawn(async move {
                let handle = b_ring;

                // Reply to the first request, giving away the serial port
                let req = cons.dequeue_async().await.map_err(drop).unwrap();
                let Request::GetPort = req.msg.body;
                let resp = req.msg.reply_with(Ok(Response::PortHandle { handle }));

                req.reply.reply_konly(resp).await.map_err(drop).unwrap();
                trace!("sent serial port handle");
                // And deny all further requests after the first
                loop {
                    let req = cons.dequeue_async().await.map_err(drop).unwrap();
                    let Request::GetPort = req.msg.body;
                    let resp = req
                        .msg
                        .reply_with(Err(SimpleSerialError::AlreadyAssignedPort));
                    req.reply.reply_konly(resp).await.map_err(drop).unwrap();
                    warn!("denied serial port handle request");
                }
            })
            .await;

        spawn_local(
            async move {
                let mut handle = a_ring;
                process_stream(&mut handle, recv, irq.clone(), recv_callback)
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
    recv_callback: fn(String),
) {
    info!("processing serial stream");
    // Wait until either the socket has data to read, or the other end of
    // the BBQueue has data to write.
    let in_stream = pin!(in_stream);
    let mut in_stream = in_stream.fuse();

    loop {
        select! {
            // The kernel wants to write something.
            mut outmsg = handle.consumer().read_grant().fuse() => {
                info!(len = outmsg.len(), "Got outgoing message");
                // TODO where do we want to send it?
                // let wall = stream.write_all(&outmsg);
                // wall.await.unwrap();
                let len = outmsg.len();
                match PortChunk::decode_from(&mut outmsg) {
                    Ok(pc) => {
                        match from_utf8(pc.chunk) {
                            Ok(s) => recv_callback(s.to_string()),
                            Err(e) => error!("could not decode PortChunk contents: {e:?}"),
                        }
                    },
                    Err(e) => error!("could not decode kernel message: {e:?}"),
                }

                outmsg.release(len);
            },
            inmsg = in_stream.next() => {
                if let Some(inmsg) = inmsg {
                    // Simulate an "interrupt", waking the kernel if it's waiting
                    // an IRQ.
                    irq.notify_one();
                    // TODO we can do better than single bytes
                    // TODO aka: use Sink trait
                    let used = 1;
                    let mut in_grant = handle.producer().send_grant_max(used).await;
                    in_grant[0] = inmsg;
                    info!(len = used, "Got incoming message",);
                    in_grant.commit(used);
                }

            }
        }
    }
}

pub fn echo(s: String) {
    let s_b = s.as_bytes();
    let chunk = PortChunk::new(WellKnown::Loopback, s.as_bytes());
    let required_size = s_b.len() + 4;
    let mut buf = vec![0; required_size];

    let mut tx = ECHO_TX.get().expect("echo tx unavailable, bruh").clone();
    match chunk.encode_to(&mut buf) {
        Ok(ser) => {
            debug!("echo: sending {ser:?}");
            for byte in ser {
                if let Err(e) = tx.try_send(*byte) {
                    tracing::error!("could not send: {e:?}");
                } else {
                    info!("echo: sent a byte!");
                }
            }
        }
        Err(e) => error!("echo: {e:?}"),
    }
}
