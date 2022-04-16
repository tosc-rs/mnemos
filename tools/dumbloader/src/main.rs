use std::{net::TcpStream, io::{Read, Write, ErrorKind}, time::Duration};
use serde::{Serialize, Deserialize};
use postcard::{CobsAccumulator, FeedResult};


#[derive(Serialize, Deserialize)]
enum Request {
    Send {
        offset: u32,
    },
    Done,
}

#[derive(Serialize, Deserialize)]
enum Response {
    Buffer {
        start: u32,
        data: Vec<u8>,
    },
    Done(u32),
    Retry,
}

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let ip = args.next().unwrap();
    let port = args.next().unwrap();
    let bin = args.next().unwrap();

    let dest = format!("{}:{}", ip, port);
    let mut conn = TcpStream::connect(&dest).unwrap();
    println!("Connected to '{}'.", dest);
    let mut file = std::fs::File::open(bin).unwrap();
    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();
    println!("Loaded file. {} bytes.", contents.len());

    while (contents.len() % 256) != 0 {
        contents.push(0xFF);
    }

    let mut acc = CobsAccumulator::<256>::new();
    let mut rdbuf = [0u8; 256];

    conn.set_read_timeout(Some(Duration::from_millis(100))).unwrap();

    'outer: loop {
        match conn.read(&mut rdbuf) {
            Ok(len) if len > 0 => {
                let mut window = &rdbuf[..len];
                while !window.is_empty() {
                    match acc.feed::<Request>(window) {
                        FeedResult::Consumed => {
                            window = &[];
                        },
                        FeedResult::OverFull(rem) => {
                            window = rem;
                            let data = postcard::to_stdvec_cobs(&Response::Retry).unwrap();
                            conn.write_all(&data).unwrap();
                        },
                        FeedResult::DeserError(rem) => {
                            window = rem;
                            let data = postcard::to_stdvec_cobs(&Response::Retry).unwrap();
                            conn.write_all(&data).unwrap();
                        },
                        FeedResult::Success { data, remaining } => {
                            window = remaining;
                            match data {
                                Request::Send { offset } => {
                                    let off_usize = offset as usize;
                                    if off_usize < contents.len() {
                                        println!("Sending 0x{:08X}", offset);
                                        let mut data = Vec::new();
                                        data.extend_from_slice(&contents[off_usize..][..256]);
                                        let data = postcard::to_stdvec_cobs(&Response::Buffer {
                                            start: offset,
                                            data,
                                        }).unwrap();
                                        conn.write_all(&data).unwrap();
                                    } else {
                                        let data = postcard::to_stdvec_cobs(&Response::Done(contents.len() as u32)).unwrap();
                                        conn.write_all(&data).unwrap();
                                    }
                                },
                                Request::Done => break 'outer,
                            }

                        },
                    }
                }
            },
            Ok(_) => {
                let data = postcard::to_stdvec_cobs(&Response::Retry).unwrap();
                conn.write_all(&data).unwrap();
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                let data = postcard::to_stdvec_cobs(&Response::Retry).unwrap();
                conn.write_all(&data).unwrap();
            }
            Err(_) => todo!(),
        }
    }

    println!("Done.");
}
