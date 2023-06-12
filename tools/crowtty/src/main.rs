use serde::{Deserialize, Serialize};
use serialport::SerialPort;
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

enum Connect {
    Serial(Box<dyn SerialPort>),
    Tcp(TcpStream),
}

impl std::io::Write for Connect {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Connect::Serial(s) => s.write(buf),
            Connect::Tcp(t) => t.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Connect::Serial(s) => s.flush(),
            Connect::Tcp(t) => t.flush(),
        }
    }
}

impl std::io::Read for Connect {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Connect::Serial(s) => s.read(buf),
            Connect::Tcp(t) => t.read(buf),
        }
    }
}

impl Connect {
    fn new_from_tcp(port: u16) -> Self {
        let port = TcpStream::connect(&format!("127.0.0.1:{port}")).unwrap();
        port.set_read_timeout(Some(Duration::from_millis(10))).ok();

        Connect::Tcp(port)
    }

    fn new_from_serial(path: &str, baud: u32) -> Self {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(10))
            .open()
            .unwrap();
        Connect::Serial(port)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();

    let mut port = match args.as_slice() {
        &["tcp", port] => {
            let port: u16 = port.parse().unwrap();
            Connect::new_from_tcp(port)
        }
        &["serial", path, baud] => {
            let baud: u32 = baud.parse().unwrap();
            Connect::new_from_serial(path, baud)
        }
        &["serial", path] => {
            Connect::new_from_serial(path, 115200)
        }
        _ => {
            println!("Args should be one of the following:");
            println!("crowtty tcp PORT          - open listener on localhost:PORT");
            println!("                            (usually 9999 for melpo)");
            println!("crowtty serial PATH       - open listener on serial port PATH w/ baud: 115200");
            println!("                            (usually /dev/ttyUSBx for hw)");
            println!("crowtty serial PATH BAUD  - open listener on serial port PATH w/ BAUD");
            println!("                            (usually /dev/ttyUSBx for hw)");
            println!("crowtty help              - this message");
            return Ok(());
        }
    };

    let mut carry = Vec::new();

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
        println!("Got {used} raw bytes");
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
