use clap::Parser;
use miette::{IntoDiagnostic, WrapErr};
use std::{fmt, path::PathBuf};

/// Boots a MnemOS x86_64 kernel using QEMU.
#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the UEFI disk image.
    ///
    /// This environment variable is set by the build script and typically does
    /// not need to be set manually.
    #[clap(long, env = "UEFI_PATH")]
    uefi_path: PathBuf,

    /// Path to the BIOS disk image.
    ///
    /// This environment variable is set by the build script and typically does
    /// not need to be set manually.
    #[clap(long, env = "BIOS_PATH")]
    bios_path: PathBuf,

    /// Path to the QEMU x86_64 executable.
    ///
    /// Generally, this does not need to be overridden, unless the QEMU binary
    /// has a non-standard name or is not on the PATH.
    #[clap(long, default_value = "qemu-system-x86_64")]
    qemu: PathBuf,

    /// Whether to boot using UEFI or BIOS.
    #[clap(long, default_value_t = BootMode::Uefi)]
    boot: BootMode,

    /// Extra arguments passed directly to the QEMU command.
    #[arg(last = true)]
    qemu_args: Vec<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, clap::ValueEnum)]
enum BootMode {
    Uefi,
    Bios,
}

// === impl Args ===

impl Args {
    pub fn run(self) -> miette::Result<()> {
        let Args {
            uefi_path,
            bios_path,
            boot,
            qemu,
            qemu_args,
        } = self;

        let mut cmd = std::process::Command::new(qemu);
        match boot {
            BootMode::Uefi => {
                let uefi_path = uefi_path.display();
                println!("booting using UEFI: {uefi_path}");

                cmd.arg("-bios").arg(ovmf_prebuilt::ovmf_pure_efi());
                cmd.arg("-drive")
                    .arg(format!("format=raw,file={uefi_path}"));
            }
            BootMode::Bios => {
                let bios_path = bios_path.display();
                println!("booting using BIOS: {bios_path}");
                cmd.arg("-drive")
                    .arg(format!("format=raw,file={bios_path}"));
            }
        }

        if !qemu_args.is_empty() {
            cmd.args(&qemu_args);
        }

        let mut child = cmd
            .spawn()
            .into_diagnostic()
            .context("failed to spawn QEMU child process")?;
        let status = child
            .wait()
            .into_diagnostic()
            .context("QEMU child process failed")?;

        if !status.success() {
            miette::bail!("QEMU exited with status: {}", status);
        }

        Ok(())
    }
}

// === impl BootMode ===

impl fmt::Display for BootMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uefi => f.write_str("uefi"),
            Self::Bios => f.write_str("bios"),
        }
    }
}
