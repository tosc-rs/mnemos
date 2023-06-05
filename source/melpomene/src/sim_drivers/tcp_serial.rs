use mnemos_kernel::{
    comms::{
        bbq::{new_bidi_channel, BidiHandle},
        kchannel::KChannel,
    },
    registry::{
        simple_serial::{Request, Response, SimpleSerial, SimpleSerialError},
        Message,
    },
    Kernel,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{self, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
};
use tracing::{info_span, trace, warn, Instrument};

pub struct TcpSerial {
    _inner: (),
}

impl TcpSerial {
    pub async fn register(
        kernel: &'static Kernel,
        ip: SocketAddr,
        incoming_size: usize,
        outgoing_size: usize,
        irq: Arc<Notify>,
    ) -> Result<(), ()> {
        let (a_ring, b_ring) = new_bidi_channel(kernel.heap(), incoming_size, outgoing_size).await;
        let (prod, cons) = KChannel::<Message<SimpleSerial>>::new_async(kernel, 2)
            .await
            .split();

        let listener = TcpListener::bind(ip).await.unwrap();
        tracing::info!("TCP serial port driver listening on {ip}");

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

        let _ = tokio::spawn(
            async move {
                let mut handle = a_ring;
                loop {
                    match listener.accept().await {
                        Ok((stream, addr)) => {
                            irq.notify_one();
                            process_stream(&mut handle, stream, irq.clone())
                                .instrument(info_span!("process_stream", client.addr = %addr))
                                .await
                        }
                        Err(error) => {
                            warn!(%error, "Error accepting incoming TCP connection");
                            return;
                        }
                    };
                }
            }
            .instrument(info_span!("TCP Serial", ?ip)),
        );

        kernel
            .with_registry(|reg| reg.register_konly::<SimpleSerial>(&prod))
            .await
            .map_err(drop)
    }
}

pub(crate) fn default_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 9999))
}

async fn process_stream(handle: &mut BidiHandle, mut stream: TcpStream, irq: Arc<Notify>) {
    loop {
        // Wait until either the socket has data to read, or the other end of
        // the BBQueue has data to write.
        tokio::select! {
            // The kernel wants to write something.
            outmsg = handle.consumer().read_grant() => {
                trace!(len = outmsg.len(), "Got outgoing message",);
                let wall = stream.write_all(&outmsg);
                wall.await.unwrap();
                let len = outmsg.len();
                outmsg.release(len);
            }
            // The socket has more bytes to read.
            _ = stream.readable() => {
                // Simulate an "interrupt", waking the kernel if it's waiting
                // an IRQ.
                irq.notify_one();
                let mut in_grant = handle.producer().send_grant_max(256).await;

                // Try to read data, this may still fail with `WouldBlock`
                // if the readiness event is a false positive.
                match stream.try_read(&mut in_grant) {
                    Ok(used) if used == 0 => {
                        warn!("Empty read, socket probably closed.");
                        return;
                    },
                    Ok(used) => {
                        trace!(len = used, "Got incoming message",);
                        in_grant.commit(used);
                    },
                    // WouldBlock here indicates that the `readable()` event was
                    // spurious. That's fine, just continue waiting for the
                    // sender to become ready or the socket to be readable again.
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue;
                    },
                    // Other errors indicate something is actually wrong.
                    Err(error) => {
                        warn!(%error, "Error reading from TCP stream");
                        return;
                    },
                }
            }
        }
    }
}
