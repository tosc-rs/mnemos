use clap::Parser;
use mnemos_x86_64_bootimager::{output, qemu, Builder};

fn main() -> miette::Result<()> {
    let App {
        cmd,
        builder,
        output,
    } = App::parse();
    output.init()?;
    tracing::info!("Assuming direct control over the build!");

    let bootimage_path = builder.build_bootimage()?;
    match cmd {
        Some(Subcommand::Build) => Ok(()),
        Some(Subcommand::Qemu(opts)) => opts.run_qemu(bootimage_path, &builder.bootloader),
        None => qemu::Options::default().run_qemu(bootimage_path, &builder.bootloader),
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
    output: output::Options,
}

#[derive(Debug, Clone, Parser)]
#[allow(clippy::large_enum_variant)] // shut up clippy, no one cares...
enum Subcommand {
    /// Just build a mnemOS boot image, and do not run it.
    Build,
    /// Build a mnemOS boot image (if needed) and launch a QEMU virtual
    /// machine to run it.
    ///
    /// This is the default subcommand.
    #[clap(alias = "run")]
    Qemu(qemu::Options),
}
