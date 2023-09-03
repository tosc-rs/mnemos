use melpo_config::TcpUartConfig;
use mnemos_kernel::{
    comms::bbq::{new_bidi_channel, BidiHandle},
    registry,
    services::simple_serial::{Request, Response, SimpleSerialError, SimpleSerialService},
    Kernel,
};
use std::sync::Arc;
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
        settings: TcpUartConfig,
        irq: Arc<Notify>,
    ) -> Result<(), registry::RegistrationError> {
        let (a_ring, b_ring) =
            new_bidi_channel(settings.incoming_size, settings.outgoing_size).await;
        let reqs = kernel
            .registry()
            .bind_konly::<SimpleSerialService>(settings.kchannel_depth)
            .await?
            .into_request_stream(settings.kchannel_depth)
            .await;
        let socket_addr = &settings.socket_addr;
        let listener = TcpListener::bind(socket_addr).await.unwrap();
        tracing::info!(
            "TCP serial port driver listening on {}",
            settings.socket_addr
        );

        kernel
            .spawn(async move {
                let handle = b_ring;

                // Reply to the first request, giving away the serial port
                let req = reqs.next_request().await;
                let Request::GetPort = req.msg.body;
                let resp = req.msg.reply_with(Ok(Response::PortHandle { handle }));

                req.reply.reply_konly(resp).await.map_err(drop).unwrap();

                // And deny all further requests after the first
                // TODO(eliza): use a connect error for this?
                loop {
                    let req = reqs.next_request().await;
                    let Request::GetPort = req.msg.body;
                    let resp = req
                        .msg
                        .reply_with(Err(SimpleSerialError::AlreadyAssignedPort));
                    req.reply.reply_konly(resp).await.map_err(drop).unwrap();
                }
            })
            .await;

        let _hdl = tokio::spawn(
            async move {
                let handle = a_ring;
                loop {
                    match listener.accept().await {
                        Ok((stream, addr)) => {
                            process_stream(&handle, stream, irq.clone())
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
            .instrument(info_span!("TCP Serial", ?socket_addr)),
        );

        Ok(())
    }
}

async fn process_stream(handle: &BidiHandle, mut stream: TcpStream, irq: Arc<Notify>) {
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
                // Simulate an "interrupt", waking the kernel if it's waiting
                // an IRQ.
                irq.notify_one();
            }
            // The socket has more bytes to read.
            _ = stream.readable() => {
                let mut in_grant = handle.producer().send_grant_max(256).await;

                // Try to read data, this may still fail with `WouldBlock`
                // if the readiness event is a false positive.
                match stream.try_read(&mut in_grant) {
                    Ok(0) => {
                        warn!("Empty read, socket probably closed.");
                        return;
                    },
                    Ok(used) => {
                        trace!(len = used, "Got incoming message",);
                        in_grant.commit(used);
                        // Simulate an "interrupt", waking the kernel if it's waiting
                        // an IRQ.
                        irq.notify_one();
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
