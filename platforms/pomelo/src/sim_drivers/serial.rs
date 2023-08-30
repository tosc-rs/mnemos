use std::{pin::pin, str::from_utf8};

use futures::{
    channel::mpsc::{self},
    select,
};
use futures_util::{FutureExt, Stream, StreamExt};
use mnemos_kernel::{
    comms::{
        bbq::{new_bidi_channel, BidiHandle},
        kchannel::KChannel,
    },
    registry::{self, Message},
    services::simple_serial::{Request, Response, SimpleSerialError, SimpleSerialService},
    Kernel,
};
use sermux_proto::{PortChunk, WellKnown};
use tracing::{debug, error, info_span, trace, warn, Instrument};

use super::io;
use crate::sim_drivers::io::irq_async;
pub struct Serial {}

impl Serial {
    pub async fn register(
        kernel: &'static Kernel,
        incoming_size: usize,
        outgoing_size: usize,
        port: u16,
        recv: mpsc::Receiver<u8>,
        recv_callback: fn(String),
    ) -> Result<(), registry::RegistrationError> {
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

        kernel
            .spawn(
                async move {
                    let handle = a_ring;
                    process_stream(&handle, recv, recv_callback)
                        .instrument(info_span!("process_stream", ?port))
                        .await
                }
                .instrument(info_span!("Serial", ?port)),
            )
            .await;
        kernel
            .with_registry(|reg| reg.register_konly::<SimpleSerialService>(&prod))
            .await
    }
}

async fn process_stream(
    handle: &BidiHandle,
    mut in_stream: impl Stream<Item = u8>,
    recv_callback: fn(String),
) {
    debug!("processing serial stream");
    // Wait until either the socket has data to read, or the other end of
    // the BBQueue has data to write.
    let in_stream = pin!(in_stream);
    let mut in_stream = in_stream.fuse();

    loop {
        select! {
            // The kernel wants to write something.
            mut outmsg = handle.consumer().read_grant().fuse() => {
                debug!(len = outmsg.len(), "Got outgoing message");
                let len = outmsg.len();
                // send all strings to `recv_callback`
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

                    // TODO we can do better than single bytes
                    // TODO aka: use Sink::send_all somehow
                    let used = 1;
                    let mut in_grant = handle.producer().send_grant_max(used).await;
                    in_grant[0] = inmsg;
                    debug!(len = used, "Got incoming message",);
                    in_grant.commit(used);

                    irq_async().await;
                }

            }
        }
    }
}

pub fn echo(s: String) {
    io::send(WellKnown::Loopback.into(), s.as_bytes());
}
