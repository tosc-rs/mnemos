use clap::Parser;
use mnemos_x86_64_bootloader::Args;

fn main() -> miette::Result<()> {
    Args::parse().run()
}
