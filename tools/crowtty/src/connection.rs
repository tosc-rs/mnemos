use libcrowtty::LogTag;
use std::path::PathBuf;
use std::{
    fmt,
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

/// Unfortunately, the `serialport` crate seems to have some issues on M-series Macs.
///
/// For these hosts, we use a patched version of the crate that has some hacky
/// fixes applied that seem to resolve the issue.
///
/// Context: <https://github.com/serialport/serialport-rs/issues/49>
mod serial {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    pub use serialport_macos_hack::*;

    #[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
    pub use serialport_regular::*;
}

use serial::SerialPort;

/// An active connection to a SerMux target.
#[derive(Debug)]
pub enum Connection {
    Serial(Box<dyn SerialPort>),
    Tcp(TcpStream),
}

/// Describes a SerMux target to connect to.
#[derive(Debug, clap::Subcommand)]
pub enum Connect {
    /// open listener on IP:PORT
    Tcp {
        /// IP address to connect to. This defaults to localhost.
        #[clap(long, default_value_t = Self::DEFAULT_IP)]
        ip: IpAddr,
        /// TCP port to connect to (usually 9999 for melpomene)
        #[arg(default_value_t = 9999)]
        port: u16,
    },
    /// open listener on PATH
    Serial {
        /// path to the serial port device (usually /dev/ttyUSBx for hw)
        path: PathBuf,

        /// baud rate (usually 115200 for hw)
        #[arg(default_value_t = Self::DEFAULT_BAUD_RATE)]
        baud: u32,
    },
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(s) => s.write(buf),
            Self::Tcp(t) => t.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Serial(s) => s.flush(),
            Self::Tcp(t) => t.flush(),
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(s) => s.read(buf),
            Self::Tcp(t) => t.read(buf),
        }
    }
}

impl Connection {
    pub fn log_tag(&self) -> LogTag {
        match self {
            Self::Serial(_) => LogTag::serial(),
            Self::Tcp(_) => LogTag::tcp(),
        }
    }
}

impl Connect {
    pub const DEFAULT_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    pub const DEFAULT_TCP_PORT: u16 = 9999;
    pub const DEFAULT_BAUD_RATE: u32 = 115200;
    const READ_TIMEOUT: Duration = Duration::from_millis(10);

    pub const fn new_tcp(port: u16) -> Self {
        Connect::Tcp {
            ip: Self::DEFAULT_IP,
            port,
        }
    }

    pub const fn default_tcp() -> Self {
        Connect::new_tcp(Self::DEFAULT_TCP_PORT)
    }

    pub fn new_serial(path: impl Into<PathBuf>) -> Self {
        Connect::Serial {
            path: path.into(),
            baud: Self::DEFAULT_BAUD_RATE,
        }
    }

    pub fn connect(&self) -> io::Result<Connection> {
        match *self {
            Self::Tcp { ip, port } => {
                let addr = SocketAddr::from((ip, port));
                let sock = TcpStream::connect(addr)?;
                sock.set_read_timeout(Some(Self::READ_TIMEOUT))?;
                Ok(Connection::Tcp(sock))
            }
            Self::Serial { ref path, baud } => {
                let path = path.to_str().ok_or_else(|| {
                    // TODO(eliza): should probably just use `Utf8PathBuf` here...
                    io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8")
                })?;
                let port = serial::new(path, baud).timeout(Self::READ_TIMEOUT).open()?;
                Ok(Connection::Serial(port))
            }
        }
    }
}

impl fmt::Display for Connect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp { ip, port } => write!(f, "{ip}:{port}"),
            Self::Serial { path, baud } => write!(f, "{} (@ {baud})", path.display()),
        }
    }
}
