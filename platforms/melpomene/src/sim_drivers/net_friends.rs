use std::{time::Duration, io::ErrorKind};

use tokio::net::{TcpStream, TcpListener, tcp::{OwnedReadHalf, OwnedWriteHalf}};

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
    let (rx, tx) = client_stream.into_split();

    let mut txc = encoder_stream(tx);
    let mut rxc = decoder_stream(rx);

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        txc.send(b"hello!".to_vec()).await.unwrap();

        let msg = rxc.recv().await.unwrap();
        let msg = core::str::from_utf8(&msg).unwrap();
        tracing::info!("Got '{msg}'");
    }
}

async fn serial_friend(host_stream: TcpStream) -> Result<(), ()> {
    let (rx, tx) = host_stream.into_split();

    let mut rxc = decoder_stream(rx);
    let mut txc = encoder_stream(tx);

    loop {
        let msg = rxc.recv().await.unwrap();
        let msg = msg.into_iter().rev().collect::<Vec<u8>>();
        txc.send(msg).await.unwrap();
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

fn encoder_stream(mut tx: OwnedWriteHalf) -> tokio::sync::mpsc::Sender<Vec<u8>> {
    let (txc, mut rxc) = tokio::sync::mpsc::channel(64);

    tokio::spawn(async move {
        loop {
            let msg: Vec<u8> = rxc.recv().await.unwrap();
            let msg = encode(msg.as_slice());

            let mut window = msg.as_slice();
            while !window.is_empty() {
                tx.writable().await.unwrap();
                match tx.try_write(window) {
                    Ok(0) => todo!(),
                    Ok(n) => {
                        window = &window[n..];
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {},
                    Err(e) => panic!("{e:?}"),
                }
            }
        }
    });

    txc
}

fn decoder_stream(mut rx: OwnedReadHalf) -> tokio::sync::mpsc::Receiver<Vec<u8>> {
    let (txc, rxc) = tokio::sync::mpsc::channel(64);

    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let mut acc = Accumulator {
            buffer: vec![],
        };

        loop {
            rx.readable().await.map_err(drop).unwrap();
            match rx.try_read(&mut buf) {
                Ok(0) => todo!(),
                Ok(n) => {
                    let mut window = &buf[..n];
                    'winny: while !window.is_empty() {
                        match acc.feed(window) {
                            AccOut::MessageRem { msg, rem } => {
                                txc.send(msg).await.unwrap();
                                window = rem;
                            },
                            AccOut::ErrorRem { rem } => todo!(),
                            AccOut::Consumed => break 'winny,
                        }
                    }
                },
                Err(e) if e.kind() == ErrorKind::WouldBlock => {},
                Err(e) => todo!("what {e:?}"),
            }
        }
    });

    rxc
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
