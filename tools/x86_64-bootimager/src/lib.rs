use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, ValueEnum, ValueHint};
use std::fmt;

pub mod output;

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
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[repr(u8)]
#[clap(rename_all = "lower")]
enum BootLogLevel {
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

#[derive(Clone, Debug, Parser)]
pub struct QemuOptions {
    /// Path to the QEMU x86_64 executable.
    ///
    /// Generally, this does not need to be overridden, unless the QEMU binary
    /// has a non-standard name or is not on the PATH.
    #[clap(
        long,
        default_value = Self::QEMU_SYSTEM_X86_64,
        value_hint = ValueHint::FilePath,
    )]
    pub qemu_path: Utf8PathBuf,

    /// Attach `crowtty` to the QEMU VM's COM1 serial port.
    #[clap(long, short)]
    pub crowtty: bool,

    /// Enable verbose output from `crowtty`.
    #[clap(long = "verbose", requires = "crowtty")]
    pub crowtty_verbose: bool,

    #[clap(flatten)]
    pub crowtty_opts: crowtty::Settings,

    /// Tracing filter to set when connecting Crowtty to the QEMU virtual serial port.
    #[clap(
        long = "serial-trace",
        alias = "serial-log",
        alias = "kernl-log",
        env = "MNEMOS_LOG",
        default_value_t = Self::default_serial_trace_filter(),
    )]
    trace_filter: tracing_subscriber::filter::Targets,

    /// Extra arguments passed directly to the QEMU command.
    #[arg(last = true)]
    pub qemu_args: Vec<String>,
}

impl Default for QemuOptions {
    fn default() -> Self {
        Self {
            qemu_path: Utf8PathBuf::from(Self::QEMU_SYSTEM_X86_64),
            qemu_args: Self::default_args(),
            crowtty: false,
            crowtty_verbose: false,
            crowtty_opts: Default::default(),
            trace_filter: Self::default_serial_trace_filter(),
        }
    }
}

impl QemuOptions {
    const QEMU_SYSTEM_X86_64: &'static str = "qemu-system-x86_64";
    fn default_serial_trace_filter() -> tracing_subscriber::filter::Targets {
        tracing_subscriber::filter::Targets::new()
            .with_default(tracing_subscriber::filter::LevelFilter::INFO)
    }

    fn default_args() -> Vec<String> {
        vec![
            "-cpu".to_string(),
            "qemu64".to_string(),
            "-smp".to_string(),
            "cores=4".to_string(),
        ]
    }

    pub fn run_qemu(
        self,
        bootimage_path: impl AsRef<Utf8Path>,
        boot_mode: BootMode,
    ) -> anyhow::Result<()> {
        use std::io::{self, Read, Write};
        use std::process::Stdio;

        let bootimage_path = bootimage_path.as_ref();
        let QemuOptions {
            qemu_path,
            qemu_args,
            crowtty,
            crowtty_opts,
            crowtty_verbose,
            trace_filter,
        } = self;

        tracing::info!(qemu = %qemu_path, args = ?qemu_args, "Booting mnemOS VM");

        let mut cmd = std::process::Command::new(qemu_path);
        if !qemu_args.is_empty() {
            cmd.args(qemu_args.iter());
        } else {
            cmd.args(QemuOptions::default_args());
        }

        cmd.arg("-drive")
            .arg(format!("format=raw,file={bootimage_path}"));

        if let BootMode::Uefi = boot_mode {
            cmd.arg("-bios").arg(ovmf_prebuilt::ovmf_pure_efi());
        }
        cmd.arg("-drive")
            .arg(format!("format=raw,file={bootimage_path}"));

        if crowtty {
            cmd.arg("-serial")
                .arg("stdio")
                .stdout(Stdio::piped())
                .stdin(Stdio::piped());
        }

        tracing::debug!(qemu = ?cmd);
        struct QemuStdio {
            stdin: std::process::ChildStdin,
            stdout: std::process::ChildStdout,
        }

        impl Read for QemuStdio {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.stdout.read(buf)
            }
        }

        impl Write for QemuStdio {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.stdin.write(buf)
            }

            fn flush(&mut self) -> io::Result<()> {
                self.stdin.flush()
            }
        }

        tracing::debug!("Running QEMU command: {cmd:?}");

        let mut qemu = cmd.spawn().context("failed to spawn QEMU child process")?;
        let crowtty_thread = if crowtty {
            let stdin = qemu.stdin.take().expect("QEMU should have piped stdin");
            let stdout = qemu.stdout.take().expect("QEMU should have piped stdout");
            Some(
                std::thread::Builder::new()
                    .name("crowtty".to_string())
                    .spawn(move || {
                        tracing::info!("Connecting crowtty...");
                        crowtty::Crowtty::new(crowtty::LogTag::serial().verbose(crowtty_verbose))
                            .settings(crowtty_opts)
                            .trace_filter(trace_filter)
                            .run(QemuStdio { stdin, stdout })
                    })
                    .unwrap(),
            )
        } else {
            None
        };

        let status = qemu.wait().context("QEMU child process failed")?;

        if !status.success() {
            anyhow::bail!("QEMU exited with status: {}", status);
        }

        if let Some(crowtty) = crowtty_thread {
            crowtty.join().unwrap().unwrap(); // TODO(eliza)
        }

        Ok(())
    }
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
            log.info = ?bootcfg.log_level,
            log.framebuffer = bootcfg.frame_buffer_logging,
            log.serial = bootcfg.serial_logging,
            "Configuring bootloader",
        );
        bootcfg
    }
}

impl Builder {
    pub fn build_bootimage(&self) -> anyhow::Result<Utf8PathBuf> {
        let t0 = std::time::Instant::now();
        tracing::info!(
            boot_mode = %self.bootloader.mode,
            kernel = %self.kernel_bin,
            "Building boot image"
        );

        let canonical_kernel_bin: Utf8PathBuf = self
            .kernel_bin
            .canonicalize()
            .context("failed to to canonicalize kernel bin path")?
            .try_into()
            .unwrap();
        let out_dir = self
            .out_dir
            .as_deref()
            .or_else(|| canonical_kernel_bin.parent())
            .ok_or_else(|| anyhow::anyhow!("can't determine OUT_DIR"))?;

        let bootcfg = self.bootloader.boot_config();
        let path = match self.bootloader.mode {
            BootMode::Uefi => {
                let path = out_dir.join("mnemos-x86_64-uefi.img");
                let mut builder = bootloader::UefiBoot::new(canonical_kernel_bin.as_ref());
                builder.set_boot_config(&bootcfg);
                builder
                    .create_disk_image(path.as_ref())
                    .map_err(|error| anyhow::anyhow!("failed to build UEFI image: {error}"))?;
                path
            }
            BootMode::Bios => {
                let path = out_dir.join("mnemos-x86_64-bios.img");
                let mut builder = bootloader::BiosBoot::new(canonical_kernel_bin.as_ref());
                builder.set_boot_config(&bootcfg);
                builder
                    .create_disk_image(path.as_ref())
                    .map_err(|error| anyhow::anyhow!("failed to build BIOS image: {error}"))?;
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
