use crate::{sim_drivers::tcp_serial, sim_tracing};
use clap::Parser;
use std::net::SocketAddr;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
pub struct Args {
    #[clap(flatten)]
    pub melpomene: MelpomeneOptions,

    #[clap(flatten)]
    pub tracing: sim_tracing::TracingOpts,
}

#[derive(Debug, clap::Args)]
pub struct MelpomeneOptions {
    /// Address to bind the TCP listener for the simulated serial port.
    #[clap(long, default_value_t = tcp_serial::default_addr())]
    pub serial_addr: SocketAddr,
}
