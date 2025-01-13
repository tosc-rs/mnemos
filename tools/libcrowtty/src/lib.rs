use clap::Parser;
use miette::{Context, IntoDiagnostic};
use owo_colors::{OwoColorize, Stream};
use sermux_proto::{DecodeError, OwnedPortChunk, WellKnown};
use std::{
    collections::HashMap,
    fmt,
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, TcpListener},
    sync::mpsc::{channel, Receiver, Sender},
    thread::{sleep, spawn, JoinHandle},
    time::{Duration, Instant},
};
use tracing::level_filters::LevelFilter;

mod keyboard;
mod trace;

pub struct Crowtty {
    settings: Settings,
    trace_filter: tracing_subscriber::filter::Targets,
    tag: LogTag,
}

#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "Crowtty Options")]
pub struct Settings {
    /// SerMux port for a pseudo-keyboard for the graphical Forth shell on the target.
    #[arg(short, long, global = true, default_value_t = sermux_proto::WellKnown::PseudoKeyboard as u16)]
    keyboard_port: u16,

    /// disables STDIN as the pseudo-keyboard.
    ///
    /// if this is set, the pseudo-keyboard port can be written to as a standard
    /// TCP port on the host, instead of reading from crowtty's STDIN.
    #[arg(long = "no-keyboard", global = true)]
    disable_stdin: bool,

    /// offset for host TCP ports.
    ///
    /// SerMux port `n` will be mapped to TCP port `n + tcp-port-base` on localhost.
    #[arg(long, global = true, default_value_t = 10_000)]
    tcp_port_base: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            keyboard_port: WellKnown::PseudoKeyboard.into(),
            disable_stdin: false,
            tcp_port_base: 10_000,
        }
    }
}

#[derive(Copy, Clone)]
pub struct LogTag {
    start: Instant,
    port: Option<u16>,
    conn: &'static str,
    pub(crate) verbose: bool,
}

impl Crowtty {
    pub fn new(tag: LogTag) -> Self {
        Self {
            settings: Default::default(),
            trace_filter: tracing_subscriber::filter::Targets::new()
                .with_default(LevelFilter::INFO),
            tag,
        }
    }

    pub fn settings(self, settings: Settings) -> Self {
        Self { settings, ..self }
    }

    pub fn trace_filter(self, filter: impl Into<tracing_subscriber::filter::Targets>) -> Self {
        Self {
            trace_filter: filter.into(),
            ..self
        }
    }

    pub fn run(self, mut port: impl Read + Write) -> miette::Result<()> {
        let Self {
            settings:
                Settings {
                    keyboard_port,
                    disable_stdin,
                    tcp_port_base,
                },
            trace_filter,
            tag,
        } = self;

        let mut carry = Vec::new();

        let mut manager = TcpManager {
            workers: HashMap::new(),
        };

        let mut host_ports = vec![WellKnown::Loopback.into(), WellKnown::HelloWorld.into()];

        if disable_stdin {
            // if the virtual keyboard is disabled, just treat the keyboard port
            // normally.
            let tag = tag.port(keyboard_port);
            println!(
                "{tag} {} pseudo-keyboard (SerMux port :{keyboard_port}) on localhost:{}",
                "KEYB".if_supports_color(Stream::Stdout, |x| x.bright_yellow()),
                keyboard_port + tcp_port_base,
            );
        } else {
            // otherwise, read from STDIN and send it to the keyboard port.
            host_ports.push(keyboard_port);
            let tag = tag.port(keyboard_port);
            println!(
                "{tag} {} pseudo-keyboard (SerMux port :{keyboard_port}) reading from STDIN",
                "KEYB".if_supports_color(Stream::Stdout, |x| x.bright_yellow()),
            );
            let handle = keyboard::KeyboardWorker::spawn(tag);
            manager.workers.insert(keyboard_port, handle);
        };

        // NOTE: You can connect to these ports using the following ncat/netcat/nc commands:
        // ```
        // # connect to port N - stdio
        // stty -icanon -echo && ncat 127.0.0.1 $PORT
        // ```
        for i in [
            WellKnown::Loopback.into(),
            WellKnown::HelloWorld.into(),
            WellKnown::ForthShell0.into(),
        ]
        .into_iter()
        {
            let (inp_send, inp_recv) = channel();
            let (out_send, out_recv) = channel();

            let socket =
                std::net::TcpListener::bind(format!("127.0.0.1:{}", tcp_port_base + i)).unwrap();

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
                        Err(e) => {
                            panic!(
                                "{tag} CONN failed to accept host connection to port {} (:{}): {e}",
                                tcp_port_base + work.port,
                                work.port
                            );
                        }
                    };

                    println!(
                        "{tag} CONN host connected to port {} (:{})",
                        tcp_port_base + work.port,
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
                                tag.if_verbose(format_args!("{mux} {n}B <- :{}", work.port));
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

        // spawn tracing listener
        let trace_port = WellKnown::BinaryTracing as u16;
        let trace_handle = {
            let (inp_send, inp_recv) = channel();
            let (out_send, out_recv) = channel::<Vec<u8>>();
            let thread_hdl = spawn(move || {
                trace::TraceWorker::new(trace_filter, inp_send, out_recv, tag.port(trace_port))
                    .run()
            });
            WorkerHandle {
                out: out_send,
                inp: inp_recv,
                _thread_hdl: thread_hdl,
            }
        };

        manager.workers.insert(trace_port, trace_handle);

        let mux = " MUX".if_supports_color(Stream::Stdout, |s| s.cyan());
        let dmux = "DMUX".if_supports_color(Stream::Stdout, |s| s.bright_purple());
        let err = "ERR!".if_supports_color(Stream::Stdout, |err| err.red());
        let text = "TEXT".if_supports_color(Stream::Stdout, |s| s.bright_yellow());
        loop {
            let mut buf = [0u8; 256];

            for (port_idx, hdl) in manager.workers.iter_mut() {
                if let Ok(msg) = hdl.inp.try_recv() {
                    let mut nmsg = Vec::new();
                    nmsg.extend_from_slice(&port_idx.to_le_bytes());
                    nmsg.extend_from_slice(&msg);
                    let mut enc_msg = cobs::encode_vec(&nmsg);
                    enc_msg.push(0);
                    tag.port(*port_idx)
                        .if_verbose(format_args!("{mux} {}B <- :{port_idx}", enc_msg.len()));
                    port.write_all(&enc_msg)
                        .into_diagnostic()
                        .with_context(|| {
                            format!(
                                "failed to write {} outbound bytes on port {port_idx}",
                                msg.len()
                            )
                        })?;
                }
            }

            let used = match port.read(&mut buf) {
                Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
                Err(e) if e.kind() == ErrorKind::TimedOut => continue,
                Ok(0) => continue,
                Ok(used) => used,
                Err(e) => return Err(e).into_diagnostic().context("inbound read failed"),
            };
            tag.if_verbose(format_args!("{mux} -> {used}B"));
            carry.extend_from_slice(&buf[..used]);

            // TODO: We probably want some kind of timeout here to force a flush
            // of the data even if we never get a null, like for example if we aren't
            // getting serial-mux data at all, and just getting plaintext with no nulls
            // at all.
            while let Some(pos) = carry.iter().position(|b| *b == 0) {
                let remainder = carry.split_off(pos + 1);

                // Success means we printed something more useful than "bad decode",
                // even if the actual decoding failed
                let mut success = false;
                match OwnedPortChunk::decode(&carry) {
                    Ok(OwnedPortChunk { port, chunk }) => {
                        success = true;
                        if let Some(hdl) = manager.workers.get_mut(&port) {
                            tag.port(port)
                                .if_verbose(format_args!("{dmux} {}B -> :{port}", chunk.len()));
                            hdl.out.send(chunk.to_vec()).ok();
                        }
                    }
                    Err(DecodeError::CobsDecodeFailed) => {
                        if let Ok(s) = std::str::from_utf8(&carry[..]) {
                            success = true;
                            for line in s.lines() {
                                println!("{tag} {text} {line}");
                            }
                        }
                    }
                    Err(DecodeError::MalformedFrame) => {
                        success = true;

                        // If the malformed frame is JUST a null terminator, this is probably
                        // a "frame flush" event, like we are just about to panic.
                        if carry != [0x00] {
                            println!("{tag} {dmux} {err} bonus data? {carry:#02x?}");
                        }
                    }
                }

                if !success {
                    println!("{tag} {dmux} {err} Bad decode!");
                }

                carry = remainder;
            }

            sleep(Duration::from_millis(10));
        }
    }
}

impl LogTag {
    pub fn serial() -> Self {
        Self::new("UART")
    }

    pub fn tcp() -> Self {
        Self::new(" TCP")
    }

    pub fn named(self, name: &'static str) -> Self {
        Self { conn: name, ..self }
    }

    pub fn verbose(self, verbose: bool) -> Self {
        Self { verbose, ..self }
    }

    pub(crate) fn new(conn: &'static str) -> Self {
        Self {
            start: Instant::now(),
            port: None,
            conn,
            verbose: false,
        }
    }

    pub(crate) fn if_verbose(&self, f: impl fmt::Display) {
        if self.verbose {
            println!("{self} {f}")
        }
    }

    pub(crate) fn port(self, port: impl Into<Option<u16>>) -> Self {
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
        // let conn = if self.tcp { " TCP" } else { "UART" };
        self.conn
            .if_supports_color(owo_colors::Stream::Stdout, |text| text.magenta())
            .fmt(f)
    }
}

struct TcpManager {
    workers: HashMap<u16, WorkerHandle>,
}

pub(crate) struct WorkerHandle {
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

pub struct Exec {
    settings: Settings,
}

impl Exec {
    pub fn new() -> Self {
        Self {
            settings: Settings::default(),
        }
    }

    pub fn settings(self, settings: Settings) -> Self {
        Self { settings }
    }

    pub fn run(self, cmd: Vec<u8>) -> Result<(), miette::Error> {
        let port = self.settings.tcp_port_base + WellKnown::ForthShell0 as u16;
        let mut stream =
            std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).into_diagnostic()?;
        eprintln!("[stderr] connected to crowtty on {:?}", stream.peer_addr());

        stream.write_all(&cmd).into_diagnostic()?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).into_diagnostic()?;

        println!("{}", String::from_utf8_lossy(&response));

        Ok(())
    }
}

impl Default for Exec {
    fn default() -> Self {
        Self::new()
    }
}
