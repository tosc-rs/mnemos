//! # MnemOS Configuration
//!
//! This crate is intended to be used by platform crates in two ways:
//!
//! ## In a `build.rs` script
//!
//! The platform crate should include this library:
//!
//! ```toml
//! [build-dependencies]
//! config = { path = "../../source/config", features = ["use-std"] }
//! ```
//!
//! And ensure your build.rs contains a call to [`buildtime::render_file`], to
//! render a single config file:
//!
//! ```rust,no_run
//! # #![allow(clippy::needless_doctest_main, non_camel_case_types)]
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct YOUR_CONFIG_TYPE(u8);
//! use mnemos_config::buildtime::render_file;
//! fn main() {
//!     // to render one config file:
//!     render_file::<YOUR_CONFIG_TYPE>("YOUR_PLATFORM.toml").unwrap();
//! }
//! ```
//!
//! To render all config files in a directory, [`buildtime::render_all`] may be
//! used instead:
//!
//! ```rust,no_run
//! # #![allow(clippy::needless_doctest_main, non_camel_case_types)]
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct YOUR_CONFIG_TYPE(u8);
//! use mnemos_config::buildtime::render_all;
//! fn main() {
//!     // to render all config files in the `board-configs` directory:
//!     render_all::<YOUR_CONFIG_TYPE>("board-configs").unwrap();
//! }
//! ```
//!
//! ## In the `main.rs`
//!
//! You'll need to include this crate *again* as a normal dependency:
//!
//! ```toml
//! [dependencies]
//! config = { path = "../../source/config" }
//! ```
//!
//! And then you can use this in your main function:
//!
//! ```rust,ignore
//! # #![allow(non_camel_case_types)]
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct YOUR_CONFIG_TYPE(u8);
//! let config = mnemos_config::include_config!(YOUR_CONFIG_TYPE).unwrap();
//! ```
//!
//! ## Make an external config crate
//!
//! In order to share data types between your platform crate and the platform
//! crate's build.rs, you should make a separate `platform-config` crate that
//! defines the shared data types.

#![cfg_attr(not(any(feature = "use-std", test)), no_std)]

use mnemos_kernel::{KernelServiceSettings, KernelSettings};
use serde::{Deserialize, Serialize};

/// The top level configuration type
///
/// This type is generic over the platform-specific configuration type
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MnemosConfig<Platform> {
    pub kernel: KernelSettings,
    pub services: KernelServiceSettings,
    pub platform: Platform,
}

pub const CONFIG_DIR_VAR: &str = "MNEMOS_CONFIG_DIR";
pub const CONFIG_FILE_VAR: &str = "MNEMOS_CONFIG";

/// Tools intended for use in build.rs scripts
#[cfg(feature = "use-std")]
pub mod buildtime {
    const OUT_DIR: &str = "OUT_DIR";
    const TAG: &str = concat!(module_path!(), ":");

    use std::{env, fs, io::Write, path::Path};

    use super::*;
    use miette::{Context, IntoDiagnostic, Result};
    use serde::de::DeserializeOwned;

    fn from_toml<Platform>(s: &str) -> Result<MnemosConfig<Platform>>
    where
        Platform: DeserializeOwned + 'static,
    {
        toml::from_str(s).into_diagnostic()
    }

    fn to_postcard<Platform>(mc: &MnemosConfig<Platform>) -> Result<Vec<u8>>
    where
        Platform: Serialize,
    {
        postcard::to_stdvec(&mc).into_diagnostic()
    }

    /// Render all configuration files in the given directory.
    ///
    /// The resulting configs are stored in the cargo `OUT_DIR`, and may be
    /// referenced by name in the main platform binary when using
    /// [`include_config!()`].
    pub fn render_all<Platform>(config_dir: impl AsRef<Path>) -> Result<()>
    where
        Platform: Serialize + DeserializeOwned + 'static,
    {
        let config_dir = config_dir.as_ref();
        let config_dir_disp = config_dir.display();
        let out_dir = env::var(OUT_DIR)
            .into_diagnostic()
            .wrap_err("Failed to read '{OUT_DIR}' env variable")?;

        println!("cargo:rerun-if-changed={config_dir_disp}");
        println!("cargo:rustc-env={CONFIG_DIR_VAR}={out_dir}");

        eprintln!("{TAG} {OUT_DIR}={out_dir}");

        (|| -> Result<()> {
            let mut rendered_any = false;
            let mut dirs = 0;
            let mut not_toml = 0;
            eprintln!("{TAG} rendering configs in '{config_dir_disp}'...");
            for entry in fs::read_dir(config_dir)
                .into_diagnostic()
                .wrap_err_with(|| {
                    format!("Failed to read config file directory '{config_dir_disp}'")
                })?
            {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        println!("cargo:warning=Error reading config dir entry: {e}");
                        continue;
                    }
                };

                let entry_path = entry.path();
                let epath_disp = match config_dir.parent() {
                    Some(parent) => entry_path
                        .strip_prefix(parent)
                        .expect("directory path must be a prefix of directory entry path?"),
                    None => entry_path.as_ref(),
                }
                .display();

                eprintln!("{TAG} file: '{epath_disp}'");

                if entry
                    .metadata()
                    .into_diagnostic()
                    .wrap_err_with(|| format!("Failed to read metadata for '{epath_disp}'"))?
                    .is_dir()
                {
                    eprintln!("{TAG}   -> not a file; skipping");
                    dirs += 1;
                    continue;
                }

                let extension = entry_path
                    .extension()
                    .map(std::ffi::OsStr::to_string_lossy)
                    .unwrap_or(std::borrow::Cow::Borrowed(""));
                if extension != "toml" {
                    eprintln!("{TAG}   -> not TOML (extension: {extension:?}); skipping");
                    not_toml += 1;
                    continue;
                }

                render_file_to::<Platform>(&entry_path, &out_dir)?;
                rendered_any = true;
            }

            if !rendered_any {
                Err(
                    miette::MietteDiagnostic::new("No config files were rendered!").with_help(
                        format!(
                        "config directory contained {not_toml} non-TOML files, {dirs} directories"
                    ),
                    ),
                )?;
            }

            Ok(())
        })()
        .wrap_err_with(|| format!("Failed to render config directory '{config_dir_disp}'"))?;

        Ok(())
    }

    /// Load a configuration file from the given path, will be made available
    /// to the main platform binary when they call [`include_config!()`].
    pub fn render_file<Platform>(path: impl AsRef<Path>) -> Result<()>
    where
        Platform: Serialize + DeserializeOwned + 'static,
    {
        let out_dir = std::env::var(OUT_DIR)
            .into_diagnostic()
            .wrap_err("Failed to read '{OUT_DIR}' env variable")?;
        eprintln!("{TAG} {OUT_DIR}='{out_dir}'");
        render_file_to::<Platform>(path, out_dir)
    }

    fn render_file_to<Platform>(path: impl AsRef<Path>, out: impl AsRef<Path>) -> Result<()>
    where
        Platform: Serialize + DeserializeOwned + 'static,
    {
        let path = path.as_ref();
        let path_disp = path.display();

        (|| {
            let filename = path
                .file_name()
                .ok_or_else(|| miette::miette!("Path has no filename!"))?;
            eprintln!("{TAG} rendering config file '{path_disp}'",);
            let cfg = std::fs::read_to_string(path).into_diagnostic()?;
            let c: MnemosConfig<Platform> = from_toml(&cfg)?;

            let mut out = out.as_ref().join(filename);
            out.set_extension("postcard");
            let bin_cfg = to_postcard(&c)?;
            let mut f = std::fs::File::create(&out).into_diagnostic()?;
            f.write_all(&bin_cfg).into_diagnostic()?;
            println!("cargo:rustc-env={CONFIG_FILE_VAR}={}", out.display());
            println!("cargo:rerun-if-changed={path_disp}");

            Ok::<_, miette::Report>(())
        })()
        .wrap_err_with(|| format!("Failed to render config file '{path_disp}'"))
    }
}

/// Tools intended for use at runtime
pub mod runtime {
    use crate::MnemosConfig;
    use serde::de::DeserializeOwned;

    #[derive(Debug, PartialEq)]
    pub enum Error {
        Postcard(postcard::Error),
    }

    pub fn from_postcard<Platform>(s: &[u8]) -> Result<MnemosConfig<Platform>, Error>
    where
        Platform: DeserializeOwned + 'static,
    {
        postcard::from_bytes(s).map_err(Error::Postcard)
    }
}

/// Load the configuration created by `render_project` in a build.rs.
///
/// Should be called with the type of your platform specific type
#[macro_export]
macro_rules! include_config {
    ($platform: ty, $name: literal) => {{
        const MNEMOS_CONFIG: &[u8] =
            include_bytes!(concat!(env!("MNEMOS_CONFIG_DIR"), "/", $name, ".postcard"));
        $crate::runtime::from_postcard::<$platform>(MNEMOS_CONFIG)
    }};
    ($platform: ty) => {{
        const MNEMOS_CONFIG: &[u8] = include_bytes!(env!("MNEMOS_CONFIG"));
        $crate::runtime::from_postcard::<$platform>(MNEMOS_CONFIG)
    }};
}
