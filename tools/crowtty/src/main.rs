use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{sleep, spawn, JoinHandle};
use std::time::Duration;

#[derive(Serialize, Deserialize)]
pub struct Chunk {
    port: u16,
    buf: Vec<u8>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let mut dport = None;

    // for port in serialport::available_ports().unwrap() {
    //     if let serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
    //         serial_number: Some(sn),
    //         ..
    //     }) = &port.port_type
    //     {
    //         if sn.as_str() == "ajm001" {
    //             dport = Some(port.clone());
    //             break;
    //         }
    //     }
    // }

    // let dport = if let Some(port) = dport {
    //     port
    // } else {
    //     eprintln!("Error: No `Pellegrino` connected!");
    //     return Ok(());
    // };

    // let mut port = serialport::new(dport.port_name, 115200)
    //     .timeout(Duration::from_millis(5))
    //     .open()
    //     .map_err(|_| "Error: failed to create port")?;

    let mut carry = Vec::new();

    let mut port = TcpStream::connect("127.0.0.1:9999").unwrap();

    // port.set_timeout(Duration::from_millis(10)).ok();
    port.set_read_timeout(Some(Duration::from_millis(10))).ok();

    let mut manager = TcpManager {
        workers: HashMap::new(),
    };

    // NOTE: You can connect to these ports using the following ncat/netcat/nc commands:
    // ```
    // # connect to port N - stdio
    // stty -icanon -echo && ncat 127.0.0.1 $PORT
    // ```
    for i in [0, 1, 2, 3].into_iter() {
        let (inp_send, inp_recv) = channel();
        let (out_send, out_recv) = channel();

        let socket = std::net::TcpListener::bind(format!("127.0.0.1:{}", 10_000 + i)).unwrap();

        let work = TcpWorker {
            out: out_recv,
            inp: inp_send,
            socket,
            port: i,
        };
        let thread_hdl = spawn(move || {
            for skt in work.socket.incoming() {
                let mut skt = match skt {
                    Ok(skt) => skt,
                    Err(_) => {
                        println!("AAAARGH");
                        panic!()
                    }
                };

                println!("Listening to port {} ({})", 10_000 + work.port, work.port);

                skt.set_read_timeout(Some(Duration::from_millis(10))).ok();
                // skt.set_nonblocking(true).ok();
                // skt.set_nodelay(true).ok();

                // let mut last = Instant::now();

                'inner: loop {
                    skt.flush().ok();
                    // if last.elapsed() >= Duration::from_millis(1000) {
                    //     last = Instant::now();
                    //     println!("Port {} says ding", work.port);
                    // }

                    if let Ok(Some(_)) = skt.take_error() {
                        println!("Took that error!");
                        break 'inner;
                    }

                    if let Ok(msg) = work.out.recv_timeout(Duration::from_millis(1)) {
                        match skt.write_all(&msg) {
                            Ok(_) => {}
                            Err(e) => {
                                println!("wtf? {:?}", e);
                                break 'inner;
                            }
                        }
                    }

                    let mut buf = [0u8; 128];
                    match skt.read(&mut buf) {
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Ok(0) | Err(_) => {
                            skt.shutdown(std::net::Shutdown::Both).ok();
                            break 'inner;
                        }
                        Ok(n) => {
                            println!("yey!");
                            work.inp.send(buf[..n].to_vec()).ok();
                        }
                    }
                }
            }
        });
        let handle = WorkerHandle {
            out: out_send,
            inp: inp_recv,
            _thread_hdl: thread_hdl,
        };

        manager.workers.insert(i, handle);
    }

    loop {
        let mut buf = [0u8; 256];

        for (port_idx, hdl) in manager.workers.iter_mut() {
            if let Ok(msg) = hdl.inp.try_recv() {
                let mut nmsg = Vec::new();
                nmsg.extend_from_slice(&port_idx.to_le_bytes());
                nmsg.extend_from_slice(&msg);
                let mut enc_msg = cobs::encode_vec(&nmsg);
                enc_msg.push(0);
                println!("Sending {} bytes to port {}", enc_msg.len(), port_idx);
                port.write_all(&enc_msg)?;
            }
        }

        let used = match port.read(&mut buf) {
            Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == ErrorKind::TimedOut => continue,
            Ok(0) => continue,
            Ok(used) => used,
            Err(e) => panic!("{:?}", e),
        };
        carry.extend_from_slice(&buf[..used]);

        while let Some(pos) = carry.iter().position(|b| *b == 0) {
            let new_chunk = carry.split_off(pos + 1);
            if let Ok(used) = cobs::decode_in_place(&mut carry) {
                let mut bytes = [0u8; 2];
                let (port, remain) = carry[..used].split_at(2);
                bytes.copy_from_slice(port);
                let port = u16::from_le_bytes(bytes);

                if let Some(hdl) = manager.workers.get_mut(&port) {
                    println!("Got {} bytes from port {}", remain.len(), port);
                    hdl.out.send(remain.to_vec()).ok();
                }
            } else {
                println!("Bad decode!");
            }
            // if let Ok(msg) = Message::decode_in_place(&mut carry) {

            // } else {
            //     println!("Bad decode!");
            // }
            carry = new_chunk;
        }

        sleep(Duration::from_millis(10));
    }
}

struct TcpManager {
    workers: HashMap<u16, WorkerHandle>,
}

struct WorkerHandle {
    out: Sender<Vec<u8>>,
    inp: Receiver<Vec<u8>>,
    _thread_hdl: JoinHandle<()>,
}

struct TcpWorker {
    out: Receiver<Vec<u8>>,
    inp: Sender<Vec<u8>>,
    port: u16,
    socket: TcpListener,
}
