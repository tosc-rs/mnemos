use owo_colors::{OwoColorize, Stream};
use serde::{Deserialize, Serialize};
use serialport::SerialPort;
use std::collections::HashMap;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{sleep, spawn, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
pub struct Chunk {
    port: u16,
    buf: Vec<u8>,
}

#[derive(Copy, Clone)]
pub(crate) struct LogTag {
    start: Instant,
    port: Option<u16>,
    tcp: bool,
}

enum Connect {
    Serial(Box<dyn SerialPort>),
    Tcp(TcpStream),
}

mod trace;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,

    /// whether to include verbose logging of bytes in/out.
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// open listener on localhost:PORT
    Tcp {
        /// TCP port to connect to (usually 9999 for melpomene)
        #[arg(default_value_t = 9999)]
        port: u16,
    },
    /// open listener on PATH
    Serial {
        /// path to the serial port device (usually /dev/ttyUSBx for hw)
        path: PathBuf,

        /// baud rate (usually 115200 for hw)
        #[arg(default_value_t = 115200)]
        baud: u32,
    },
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
    let args = Args::parse();
    let verbose = args.verbose;
    let (mut port, tag) = match args.command {
        Command::Tcp { port } => (Connect::new_from_tcp(port), LogTag::new(true)),
        Command::Serial { path, baud } => (
            Connect::new_from_serial(path.to_str().unwrap(), baud),
            LogTag::new(false),
        ),
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
    for i in [0, 1, 2].into_iter() {
        let (inp_send, inp_recv) = channel();
        let (out_send, out_recv) = channel();

        let socket = std::net::TcpListener::bind(format!("127.0.0.1:{}", 10_000 + i)).unwrap();

        let work = TcpWorker {
            out: out_recv,
            inp: inp_send,
            socket,
            port: i,
        };
        let tag = tag.port(i);
        let thread_hdl = spawn(move || {
            let mux = " MUX".if_supports_color(Stream::Stdout, |s| s.cyan());
            let dmux = "DMUX".if_supports_color(Stream::Stdout, |s| s.bright_purple());
            let err = "ERR!".if_supports_color(Stream::Stdout, |err| err.red());
            for skt in work.socket.incoming() {
                let mut skt = match skt {
                    Ok(skt) => skt,
                    Err(_) => {
                        println!("AAAARGH");
                        panic!()
                    }
                };

                println!(
                    "{tag} CONN host connected to port {} (:{})",
                    10_000 + work.port,
                    work.port
                );

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

                    if let Ok(Some(e)) = skt.take_error() {
                        println!("{tag} {mux} {err} {e}");
                        break 'inner;
                    }

                    if let Ok(msg) = work.out.recv_timeout(Duration::from_millis(1)) {
                        match skt.write_all(&msg) {
                            Ok(_) => {}
                            Err(e) => {
                                println!("{tag} {dmux} {err} write error: {e}");
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
                            if verbose {
                                println!("{tag} {mux} {n}B <- :{}", work.port);
                            }
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

    // spawn tracing on port 3
    let trace_handle = {
        let (inp_send, inp_recv) = channel();
        let (out_send, out_recv) = channel::<Vec<u8>>();
        let thread_hdl = spawn(move || {
            // don't drop this
            let _inp_send = inp_send;
            trace::decode(out_recv, tag.port(3), verbose)
        });
        WorkerHandle {
            out: out_send,
            inp: inp_recv,
            _thread_hdl: thread_hdl,
        }
    };

    manager.workers.insert(3, trace_handle);

    let mux = " MUX".if_supports_color(Stream::Stdout, |s| s.cyan());
    let dmux = "DMUX".if_supports_color(Stream::Stdout, |s| s.bright_purple());
    let err = "ERR!".if_supports_color(Stream::Stdout, |err| err.red());
    loop {
        let mut buf = [0u8; 256];

        for (port_idx, hdl) in manager.workers.iter_mut() {
            if let Ok(msg) = hdl.inp.try_recv() {
                let mut nmsg = Vec::new();
                nmsg.extend_from_slice(&port_idx.to_le_bytes());
                nmsg.extend_from_slice(&msg);
                let mut enc_msg = cobs::encode_vec(&nmsg);
                enc_msg.push(0);
                if verbose {
                    println!(
                        "{} {mux} {}B <- :{port_idx}",
                        tag.port(*port_idx),
                        enc_msg.len()
                    );
                }
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
        if verbose {
            println!("{tag} {mux} -> {used}B");
        }
        carry.extend_from_slice(&buf[..used]);

        while let Some(pos) = carry.iter().position(|b| *b == 0) {
            let new_chunk = carry.split_off(pos + 1);
            if let Ok(used) = cobs::decode_in_place(&mut carry) {
                let mut bytes = [0u8; 2];
                let (port, remain) = carry[..used].split_at(2);
                bytes.copy_from_slice(port);
                let port = u16::from_le_bytes(bytes);

                if let Some(hdl) = manager.workers.get_mut(&port) {
                    if verbose {
                        println!("{} {dmux} {}B -> :{port}", tag.port(port), remain.len());
                    }
                    hdl.out.send(remain.to_vec()).ok();
                }
            } else {
                println!("{} {dmux} {err} Bad decode!", tag);
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

impl LogTag {
    pub fn new(tcp: bool) -> Self {
        Self {
            start: Instant::now(),
            port: None,
            tcp,
        }
    }

    pub fn port(self, port: impl Into<Option<u16>>) -> Self {
        Self {
            port: port.into(),
            ..self
        }
    }
}

impl fmt::Display for LogTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let elapsed = self.start.elapsed();
        let port = self
            .port
            .as_ref()
            .map(|p| p as &dyn fmt::Display)
            .unwrap_or(&" " as &dyn fmt::Display);
        format_args!(
            "[{port} +{:04}.{:09}s] ",
            elapsed.as_secs(),
            elapsed.subsec_nanos()
        )
        .if_supports_color(owo_colors::Stream::Stdout, |text| text.dimmed())
        .fmt(f)?;
        let conn = if self.tcp { " TCP" } else { "UART" };
        conn.if_supports_color(owo_colors::Stream::Stdout, |text| text.magenta())
            .fmt(f)
    }
}
