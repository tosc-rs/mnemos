use crate::sim_tracing;
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
pub struct Args {
    #[clap(flatten)]
    pub tracing: sim_tracing::TracingOpts,
}
