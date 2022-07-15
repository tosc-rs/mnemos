use mnemos_kernel::comms::bbq::BidiHandle;
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time,
};
use tracing::{info_span, trace, warn, Instrument};

pub async fn spawn_tcp_serial(handle: BidiHandle) {
    let ip = SocketAddr::from(([127, 0, 0, 1], 9999));
    let listener = TcpListener::bind(ip).await.unwrap();
    let _ = tokio::spawn(
        async move {
            let mut handle = handle;
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        process_stream(&mut handle, stream)
                            .instrument(info_span!("process_stream", client.addr = %addr))
                            .await
                    }
                    Err(error) => {
                        warn!(%error,
                            "error accepting incoming TCP connection"
                        );
                        return;
                    }
                };
            }
        }
        .instrument(info_span!("TCP Serial", ?ip)),
    );
}

async fn process_stream(handle: &mut BidiHandle, mut stream: TcpStream) {
    const READ_TIMEOUT: time::Duration = time::Duration::from_millis(25);
    loop {
        // TODO(eliza): it would be nice to have separate tasks waiting for
        // reads/writes...
        tokio::select! {
            outmsg = handle.consumer().read_grant() => {
                trace!(len = outmsg.len(), "Got outgoing message",);
                let wall = stream.write_all(&outmsg);
                wall.await.unwrap();
                let len = outmsg.len();
                outmsg.release(len);
            }
            mut in_grant = handle.producer().send_grant_max(256) => {
                match time::timeout(READ_TIMEOUT, stream.read(&mut in_grant)).await {
                    Ok(Ok(used)) if used == 0 => {
                        warn!("Empty read, socket probably closed.");
                        return;
                    }
                    Ok(Ok(used)) => {
                        trace!(len = used, "Got incoming message",);
                        in_grant.commit(used);
                    }
                    // The outer error indicates a timeout; that's fine, just
                    // nothing to read right now.
                    Err(_) => {
                        trace!("read timed out after {:?}", READ_TIMEOUT);
                    }
                    // The inner error indicates that the read actually failed.
                    Ok(Err(error)) => {
                        warn!(%error, "error reading from TCP stream");
                        return;
                    }
                }

            }
        }
    }
}
