use clap::{Args, Parser};
use mnemos_x86_64_bootimager::{Builder, QemuOptions};

fn main() -> anyhow::Result<()> {
    use tracing_subscriber::prelude::*;

    let App {
        cmd,
        builder,
        output,
    } = App::parse();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().without_time().pretty())
        .with(output.trace_filter)
        .init();

    let bootimage_path = builder.build_bootimage()?;
    let mode = builder.bootloader.mode;
    match cmd {
        Some(Subcommand::Build) => Ok(()),
        Some(Subcommand::Qemu(opts)) => opts.run_qemu(bootimage_path, mode),
        None => QemuOptions::default().run_qemu(bootimage_path, mode),
    }
}

/// A tool to build and run a bootable mnemOS x86_64 disk image.
#[derive(Debug, Parser)]
#[clap(about, version, author = "Eliza Weisman <eliza@elizas.website>")]
struct App {
    /// Which subcommand to run?
    ///
    /// If none is present, this defaults to 'qemu'.
    #[clap(subcommand)]
    cmd: Option<Subcommand>,

    /// Builder configuration for actually making the bootimage.
    #[clap(flatten)]
    builder: Builder,

    #[clap(flatten)]
    output: OutputOptions,
}

#[derive(Debug, Clone, Parser)]
enum Subcommand {
    /// Just build a mnemOS boot image, and do not run it.
    Build,
    /// Build a mnemOS boot image (if needed) and launch a QEMU virtual
    /// machine to run it.
    ///
    /// This is the default subcommand.
    #[clap(alias = "run")]
    Qemu(QemuOptions),
}

#[derive(Clone, Debug, Args)]
#[command(next_help_heading = "Output Options")]
struct OutputOptions {
    /// Tracing filter for the bootimage builder.
    #[clap(
        long = "trace",
        alias = "log",
        env = "RUST_LOG",
        default_value = "info",
        global = true
    )]
    trace_filter: tracing_subscriber::filter::Targets,
}
