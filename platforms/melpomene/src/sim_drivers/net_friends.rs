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
    let mut acc = Accumulator {
        buffer: vec![],
    };
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        tx.writable().await.map_err(drop)?;
        tx.try_write(&encode(b"hello!")).map_err(drop)?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        rx.readable().await.map_err(drop)?;
        match rx.try_read(&mut buf) {
            Ok(0) => todo!(),
            Ok(n) => {
                let mut window = &buf[..n];
                'winny: while !window.is_empty() {
                    match acc.feed(window) {
                        AccOut::MessageRem { msg, rem } => {
                            let msg = core::str::from_utf8(&msg).unwrap();
                            tracing::info!("Decoded '{msg}'");
                            window = rem;
                        },
                        AccOut::ErrorRem { rem } => todo!(),
                        AccOut::Consumed => break 'winny,
                    }
                }
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

struct Accumulator {
    buffer: Vec<u8>,
}

enum AccOut<'a> {
    MessageRem {
        msg: Vec<u8>,
        rem: &'a [u8],
    },
    ErrorRem {
        rem: &'a [u8],
    },
    Consumed,
}

fn encode(src: &[u8]) -> Vec<u8> {
    cobs::encode_vec(src)
}

impl Accumulator {
    fn feed<'a>(&mut self, buf: &'a [u8]) -> AccOut<'a> {
        match buf.split_inclusive(|n| *n == 0).next() {
            Some(rel) => {
                let rel_len = rel.len();
                self.buffer.extend_from_slice(rel);
                let out = cobs::decode_vec(&self.buffer);
                self.buffer.clear();
                match out {
                    Ok(v) => AccOut::MessageRem { msg: v, rem: &buf[rel_len..] },
                    Err(_) => AccOut::ErrorRem { rem: &buf[rel_len..] },
                }
            },
            None => {
                self.buffer.extend_from_slice(buf);
                AccOut::Consumed
            },
        }
    }
}
