use camino::Utf8PathBuf;
use clap::{Args, Parser, ValueEnum, ValueHint};
use miette::{miette, Context, IntoDiagnostic};
use std::fmt;

pub mod output;
pub mod qemu;

#[derive(Debug, Parser)]
#[command(next_help_heading = "Build Options")]
pub struct Builder {
    /// The path to the kernel binary.
    #[clap(long, short = 'k', value_hint = ValueHint::FilePath)]
    pub kernel_bin: Utf8PathBuf,

    /// Overrides the directory in which to build the output image.
    #[clap(
        long,
        env = "OUT_DIR",
        value_hint = ValueHint::DirPath,
        global = true,
    )]
    pub out_dir: Option<Utf8PathBuf>,

    /// Overrides the target directory for the kernel build.
    #[clap(
        long,
        env = "CARGO_TARGET_DIR",
        value_hint = ValueHint::DirPath,
        global = true
    )]
    pub target_dir: Option<Utf8PathBuf>,

    /// Overrides the path to the `cargo` executable.
    ///
    /// By default, this is read from the `CARGO` environment variable.
    #[clap(
        long = "cargo",
        env = "CARGO",
        default_value = "cargo",
        value_hint = ValueHint::ExecutablePath,
        global = true
    )]
    pub cargo_path: Utf8PathBuf,

    /// Configures the bootloader.
    #[clap(flatten)]
    pub bootloader: BootloaderOptions,
}

#[derive(Clone, Debug, Args)]
#[command(next_help_heading = "Bootloader Options")]
pub struct BootloaderOptions {
    /// How to boot mnemOS.
    ///
    /// This determines which type of image is built, and (if a QEMU subcommand
    /// is executed) how QEMU will boot mnemOS.
    #[clap(
        long = "boot",
        short = 'b',
        default_value_t = BootMode::Uefi,
        global = true,
    )]
    pub mode: BootMode,

    /// Log level for the bootloader.
    #[clap(
        long,
        default_value_t = BootLogLevel::Debug,
        global = true,
    )]
    boot_log: BootLogLevel,

    /// Instructs the bootloader to set up a framebuffer format that
    ///  has at least the given height.
    ///
    /// If this is not possible, the bootloader will fall back to a
    /// smaller format.
    #[clap(long, global = true)]
    framebuffer_height: Option<u64>,

    /// Instructs the bootloader to set up a framebuffer format that has at
    /// least the given width.
    ///
    /// If this is not possible, the bootloader will fall back to a smaller
    /// format.
    #[clap(long, global = true)]
    framebuffer_width: Option<u64>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[repr(u8)]
#[clap(rename_all = "upper")]
pub enum BootMode {
    /// Boot mnemOS using the UEFI bootloader.
    ///
    /// The kernel image will be output to `<OUT_DIR>/mnemos-uefi.img`.
    Uefi,
    /// Boot mnemOS using the legacy BIOS bootloader.
    ///
    /// The kernel image will be output to `<OUT_DIR>/mnemos-bios.img`.
    Bios,
}

impl fmt::Display for BootMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Uefi => "UEFI",
            Self::Bios => "BIOS",
        })
    }
}

/// Log levels for the `bootloader` crate.
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, PartialOrd, Ord)]
#[repr(u8)]
#[clap(rename_all = "lower")]
pub(crate) enum BootLogLevel {
    /// A level lower than all log levels.
    Off,
    /// Corresponds to the `Error` log level.
    Error,
    /// Corresponds to the `Warn` log level.
    Warn,
    /// Corresponds to the `Info` log level.
    Info,
    /// Corresponds to the `Debug` log level.
    Debug,
    /// Corresponds to the `Trace` log level.
    Trace,
}

impl fmt::Display for BootLogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        })
    }
}

// === impl BootloaderOptions ===

impl BootloaderOptions {
    fn boot_config(&self) -> bootloader_boot_config::BootConfig {
        use bootloader_boot_config::LevelFilter;
        let mut bootcfg = bootloader::BootConfig::default();
        bootcfg.log_level = match self.boot_log {
            BootLogLevel::Off => LevelFilter::Off,
            BootLogLevel::Error => LevelFilter::Error,
            BootLogLevel::Warn => LevelFilter::Warn,
            BootLogLevel::Info => LevelFilter::Info,
            BootLogLevel::Debug => LevelFilter::Debug,
            BootLogLevel::Trace => LevelFilter::Trace,
        };
        bootcfg.frame_buffer_logging = true;
        bootcfg.serial_logging = true;
        if self.framebuffer_height.is_some() {
            bootcfg.frame_buffer.minimum_framebuffer_height = self.framebuffer_height;
        }
        if self.framebuffer_width.is_some() {
            bootcfg.frame_buffer.minimum_framebuffer_width = self.framebuffer_width;
        }
        tracing::debug!(
            ?bootcfg.log_level,
            bootcfg.frame_buffer_logging,
            bootcfg.serial_logging,
            "Configuring bootloader",
        );
        bootcfg
    }
}

impl Builder {
    pub fn build_bootimage(&self) -> miette::Result<Utf8PathBuf> {
        let t0 = std::time::Instant::now();
        tracing::info!(
            boot_mode = %self.bootloader.mode,
            kernel = %self.kernel_bin,
            "Building boot image"
        );

        let canonical_kernel_bin: Utf8PathBuf = self
            .kernel_bin
            .canonicalize()
            .into_diagnostic()
            .context("failed to to canonicalize kernel bin path")?
            .try_into()
            .unwrap();
        let out_dir = self
            .out_dir
            .as_deref()
            .or_else(|| canonical_kernel_bin.parent())
            .ok_or_else(|| miette!("can't determine OUT_DIR"))?;

        let bootcfg = self.bootloader.boot_config();
        let path = match self.bootloader.mode {
            BootMode::Uefi => {
                let path = out_dir.join("mnemos-x86_64-uefi.img");
                let mut builder = bootloader::UefiBoot::new(canonical_kernel_bin.as_ref());
                builder.set_boot_config(&bootcfg);
                builder
                    .create_disk_image(path.as_ref())
                    .map_err(|error| miette!("failed to build UEFI image: {error}"))?;
                path
            }
            BootMode::Bios => {
                let path = out_dir.join("mnemos-x86_64-bios.img");
                let mut builder = bootloader::BiosBoot::new(canonical_kernel_bin.as_ref());
                builder.set_boot_config(&bootcfg);
                builder
                    .create_disk_image(path.as_ref())
                    .map_err(|error| miette!("failed to build BIOS image: {error}"))?;
                path
            }
        };

        tracing::info!(
            "Finished bootable disk image [{}] in {:.02?} ({path})",
            self.bootloader.mode,
            t0.elapsed(),
        );

        Ok(path)
    }
}
