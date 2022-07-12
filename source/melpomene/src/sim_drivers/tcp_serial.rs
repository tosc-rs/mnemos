use mnemos_kernel::comms::bbq::BidiHandle;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{sleep, spawn};
use std::time::Duration;
use tracing::{trace, warn};

pub fn spawn_tcp_serial(handle: BidiHandle) {
    let listener = TcpListener::bind("127.0.0.1:9999").unwrap();
    let _ = spawn(move || {
        let mut handle = handle;
        for stream in listener.incoming() {
            process_stream(&mut handle, stream.unwrap());
        }
    });
}

fn process_stream(handle: &mut BidiHandle, mut stream: TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_millis(25)))
        .unwrap();
    loop {
        if let Some(outmsg) = handle.consumer().read_grant_sync() {
            trace!(len = outmsg.len(), "Got outgoing message",);
            stream.write_all(&outmsg).unwrap();
            let len = outmsg.len();
            outmsg.release(len);
        }

        if let Some(mut in_grant) = handle.producer().send_grant_max_sync(256) {
            match stream.read(&mut in_grant) {
                Ok(used) if used == 0 => {
                    warn!("Empty read, socket probably closed.");
                    return;
                }
                Ok(used) => {
                    trace!(len = used, "Got incoming message",);
                    in_grant.commit(used);
                }
                Err(e) if e.kind() == ErrorKind::TimedOut => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => panic!("stream: {:?}", e),
            }
        }

        sleep(Duration::from_millis(25));
    }
}
