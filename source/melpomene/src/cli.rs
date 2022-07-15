use clap::Parser;
use std::net::SocketAddr;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
pub struct Args {
    /// Address to bind the TCP listener for the simulated serial port.
    #[clap(long)]
    pub serial_addr: SocketAddr,

    #[clap(flatten)]
    pub tracing: crate::sim_tracing::TracingOpts,
}
