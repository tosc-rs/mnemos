fn main() -> anyhow::Result<()> {
    #[cfg(feature = "_any_deps")]
    install_deps()?;

    Ok(())
}

#[cfg(feature = "_any_deps")]
fn install_deps() -> anyhow::Result<()> {
    use anyhow::{Context, Result};
    use std::{
        env, fs,
        path::{Path, PathBuf},
    };

    fn install_bin(bins_path: &impl AsRef<Path>, pkg: &str, bin: &str) -> Result<()> {
        // set by cargo's artifact dependency feature, see
        // https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#artifact-dependencies
        let artifact_path = path_from_env(&format!("CARGO_BIN_FILE_{pkg}_{bin}"))?;
        symlink_file(artifact_path, bins_path.as_ref().join(bin))
            .with_context(|| format!("failed to symlink {bin}"))?;
        Ok(())
    }

    fn path_from_env(var: &str) -> Result<PathBuf> {
        env::var_os(var)
            .ok_or_else(|| anyhow::anyhow!("environment variable `{var}` not set"))
            .map(PathBuf::from)
    }

    fn symlink_file(original: impl AsRef<Path>, link: impl AsRef<Path>) -> Result<()> {
        #[cfg(unix)]
        use std::os::unix::fs::symlink as symlink_file_os;

        #[cfg(windows)]
        use std::os::windows::fs::symlink_file_os;

        let original = original.as_ref();
        let link = link.as_ref();

        if link.exists() {
            fs::remove_file(link).with_context(|| {
                format!("failed to remove existing simlink at {}", link.display())
            })?;
        }

        symlink_file_os(original, link).with_context(|| {
            format!(
                "failed to create symlink:\nsrc: {}\ndst: {}",
                original.display(),
                link.display()
            )
        })?;

        Ok(())
    }

    // set by cargo, build scripts should use this directory for output files
    let out_dir = path_from_env("OUT_DIR")?;

    let bins_path = out_dir.join("manganese-bins");

    fs::create_dir_all(&bins_path)
        .with_context(|| format!("failed to create bins directory {}", bins_path.display()))?;

    if cfg!(feature = "cargo-nextest") {
        install_bin(&bins_path, "CARGO_NEXTEST", "cargo-nextest")?;
    }

    if cfg!(feature = "cargo-binutils") {
        // TODO(eliza): should we also add the other cargo-binutils tools to the bin path?
        install_bin(&bins_path, "CARGO_BINUTILS", "cargo-objcopy")?;
    }

    if cfg!(feature = "cargo-espflash") {
        install_bin(&bins_path, "CARGO_ESPFLASH", "cargo-espflash")?;
    }

    if cfg!(feature = "trunk") {
        install_bin(&bins_path, "TRUNK", "trunk")?;
    }

    install_bin(&bins_path, "JUST", "just")?;

    println!("cargo:rustc-env=MN_CARGO_BINS={}", bins_path.display());

    Ok(())
}
