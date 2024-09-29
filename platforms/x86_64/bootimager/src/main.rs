use clap::Parser;
use mnemos_x86_64_bootimager::{Bootimager, Options};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    Bootimager::from_options(Options::parse())?.run()
}
