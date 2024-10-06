use crate::{BootLogLevel, BootMode, BootloaderOptions};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, ValueHint};
use miette::{Context, IntoDiagnostic};
use std::io::{self, BufRead, BufReader, Read, Write};

#[derive(Clone, Debug, Parser)]
pub struct Options {
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

    /// If true, do not attach `crowtty` to the QEMU VM's COM1 serial port.
    ///
    /// Crowtty will also be disabled if the QEMU command line contains an
    /// explicit `-serial` flag, indicating the user wants to do somethign else
    /// with QEMU's virtual serial port.
    #[clap(long, short)]
    pub no_crowtty: bool,

    /// Enable verbose output from `crowtty`.
    #[clap(long = "verbose", conflicts_with = "no-crowtty")]
    pub crowtty_verbose: bool,

    #[clap(flatten)]
    pub crowtty_opts: crowtty::Settings,

    /// Tracing filter to set when connecting Crowtty to the QEMU virtual serial
    /// port.
    #[clap(
        long = "serial-trace",
        alias = "serial-log",
        alias = "kernel-log",
        env = "MNEMOS_LOG",
        default_value_t = Self::default_serial_trace_filter(),
    )]
    trace_filter: tracing_subscriber::filter::Targets,

    /// Extra arguments passed directly to the QEMU command.
    #[arg(last = true)]
    pub qemu_args: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            qemu_path: Utf8PathBuf::from(Self::QEMU_SYSTEM_X86_64),
            qemu_args: Self::default_args(),
            no_crowtty: false,
            crowtty_verbose: false,
            crowtty_opts: Default::default(),
            trace_filter: Self::default_serial_trace_filter(),
        }
    }
}

impl Options {
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
        boot_opts: &BootloaderOptions,
    ) -> miette::Result<()> {
        use std::process::Stdio;

        let bootimage_path = bootimage_path.as_ref();

        tracing::info!(qemu = %self.qemu_path, args = ?self.qemu_args, "Booting mnemOS VM");

        let mut cmd = std::process::Command::new(self.qemu_path);
        if !self.qemu_args.is_empty() {
            cmd.args(self.qemu_args.iter());
        } else {
            cmd.args(Options::default_args());
        }

        cmd.arg("-drive")
            .arg(format!("format=raw,file={bootimage_path}"));

        if let BootMode::Uefi = boot_opts.mode {
            cmd.arg("-bios").arg(ovmf_prebuilt::ovmf_pure_efi());
        }

        let crowtty_enabled =
            // Disable crowtty if the user explicitly asked for it to be disabled.
            !self.no_crowtty &&
            // Disable crowtty if the user is trying to do something else with the
            // serial port.
            self.qemu_args.iter().all(|arg| arg != "-serial");

        if crowtty_enabled {
            cmd.arg("-serial")
                .arg("stdio")
                .stdout(Stdio::piped())
                .stdin(Stdio::piped());
        }

        cmd.stderr(Stdio::piped());

        tracing::debug!("Running QEMU command: {cmd:?}");

        let mut qemu = cmd
            .spawn()
            .into_diagnostic()
            .context("failed to spawn QEMU child process")?;

        let tag = crowtty::LogTag::serial().verbose(self.crowtty_verbose);
        let crowtty_thread = if crowtty_enabled {
            let stdin = qemu.stdin.take().expect("QEMU should have piped stdin");
            let stdout = qemu.stdout.take().expect("QEMU should have piped stdout");
            let boot_log = boot_opts.boot_log;

            let thread = std::thread::Builder::new()
                .name("crowtty".to_string())
                .spawn(move || {
                    run_crowtty(
                        tag,
                        self.crowtty_opts,
                        self.trace_filter,
                        boot_log,
                        stdin,
                        stdout,
                    )
                })
                .unwrap();
            Some(thread)
        } else {
            None
        };

        let qemu_stderr = {
            let stderr = qemu.stderr.take().expect("QEMU should have piped stderr");
            BufReader::new(stderr)
        };
        let qemu_tag = tag.named("QEMU");
        for line in qemu_stderr.lines() {
            match line {
                Ok(line) => eprintln!("{qemu_tag} {line:?}"),
                Err(error) => {
                    tracing::warn!(%error, "failed to read from QEMU stderr");
                    break;
                }
            }
        }

        let status = qemu
            .wait()
            .into_diagnostic()
            .context("QEMU child process failed")?;

        if !status.success() {
            return Err(miette::miette!("QEMU exited with {status}"));
        }

        if let Some(crowtty) = crowtty_thread {
            crowtty.join().unwrap()?;
        }

        Ok(())
    }
}

fn run_crowtty(
    tag: crowtty::LogTag,
    crowtty_opts: crowtty::Settings,
    trace_filter: tracing_subscriber::filter::Targets,
    boot_log: BootLogLevel,
    stdin: std::process::ChildStdin,
    stdout: std::process::ChildStdout,
) -> miette::Result<()> {
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

    tracing::info!("Connecting crowtty...");
    // The bootloader will log a bunch of stuff to serial that isn't formatted
    // in mnemOS' trace proto. Unfortunately, crowtty will just silently eat
    // these logs until it sees a 0 byte, which the kernel may not send (e.g. if
    // the serial port is disabled, or if the kernel fails to come up).
    // Therefore, before we actually start crowtty, consume all the logs from
    // the bootloader before we see the line that indicates it's jumped to the
    // real life kernel.
    //
    // TODO(eliza): a nicer solution might be to make crowtty smarter about
    // handling random non-sermux ASCII characters when it's not inside of a
    let stdout = if boot_log >= BootLogLevel::Info {
        let mut stdout = BufReader::new(stdout);
        for line in stdout.by_ref().lines() {
            match line {
                Ok(line) => {
                    // OVMF thinks it's so smart for sending a bunch of escape
                    // codes as soon as it comes up. But we would really prefer
                    // not to clear the terminal etc.
                    let ovmf_garbage = "\u{1b}[2J\u{1b}[01;01H\u{1b}[=3h\u{1b}[2J\u{1b}[01;01H";
                    let line = line.trim_start_matches(ovmf_garbage);
                    // Any remaining control characters must be inserted
                    // directly into the trash can.
                    let line = line
                        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
                        .trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
                    eprintln!("{tag} BOOT {line}");
                    if line.contains("Jumping to kernel") {
                        break;
                    }
                }

                Err(error) => {
                    tracing::warn!(%error, "failed to read from QEMU stdout");
                    break;
                }
            }
        }
        stdout.into_inner()
    } else {
        stdout
    };

    crowtty::Crowtty::new(tag)
        .settings(crowtty_opts)
        .trace_filter(trace_filter)
        .run(QemuStdio { stdin, stdout })
}
