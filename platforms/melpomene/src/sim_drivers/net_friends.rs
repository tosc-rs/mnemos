use std::{time::Duration, io::ErrorKind};

use tokio::net::{TcpStream, TcpListener};

pub struct NetFriends {

}

pub struct FakeSerialFriend {
    pub friend: tokio::task::JoinHandle<Result<(), ()>>,
    pub client: tokio::task::JoinHandle<Result<(), ()>>,
    pub listener: tokio::net::TcpListener,
}

impl FakeSerialFriend {
    pub async fn new(port: u16) -> Self {
        let addr = format!("localhost:{port}");
        let host_listener = TcpListener::bind(&addr).await.unwrap();
        let client = TcpStream::connect(&addr).await.unwrap();
        let (host_stream, _client_addr) = host_listener.accept().await.unwrap();
        FakeSerialFriend {
            friend: tokio::spawn(serial_friend(host_stream)),
            client: tokio::spawn(friend_talker(client)),
            listener: host_listener,
        }
    }
}

async fn friend_talker(client_stream: TcpStream) -> Result<(), ()> {
    let (mut rx, mut tx) = client_stream.into_split();
    let mut buf = [0u8; 4096];
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        tx.writable().await.map_err(drop)?;
        tx.try_write(b"hello!").map_err(drop)?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        rx.readable().await.map_err(drop)?;
        match rx.try_read(&mut buf) {
            Ok(0) => todo!(),
            Ok(n) => {
                let msg = core::str::from_utf8(&buf[..n]).unwrap();
                tracing::info!("Client got: {msg}");
            },
            Err(e) => todo!("what {e:?}"),
        }
    }
}

async fn serial_friend(host_stream: TcpStream) -> Result<(), ()> {
    let (mut rx, mut tx) = host_stream.into_split();

    // loopback
    let mut buf = [0u8; 4096];
    loop {
        rx.readable().await.map_err(drop)?;
        match rx.try_read(&mut buf) {
            Ok(0) => todo!("rx hung up"),
            Ok(n) => {
                tracing::info!("Friend got {n} bytes");
                let mut window = &buf[..n];
                while !window.is_empty() {
                    tx.writable().await.map_err(drop)?;
                    match tx.try_write(window) {
                        Ok(0) => todo!("tx hung up"),
                        Ok(n) => {
                            tracing::info!("Friend sent {n} bytes");
                            window = &window[n..];
                        }
                        Err(e) => panic!("what {e:?}"),
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {},
            Err(e) => panic!("what {e:?}"),
        }
    }

    Err(())
}
