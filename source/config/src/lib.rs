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
//! And ensure your build.rs contains:
//!
//! ```rust,skip
//! # #![allow(clippy::needless_doctest_main)]
//! use mnemos_config::buildtime::render_project;
//! fn main() {
//!     render_project::<YOUR_CONFIG_TYPE>("YOUR_PLATFORM.toml").unwrap();
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
//! ```rust,skip
//! let config = mnemos_config::load_configuration!(YOUR_CONFIG_TYPE).unwrap();
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

/// Tools intended for use in build.rs scripts
#[cfg(feature = "use-std")]
pub mod buildtime {
    use std::{io::Write, path::PathBuf};

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

    /// Load a configuration file from the given path, will be made available
    /// to the main platform binary when they call [load_configuration!()].
    pub fn render_project<Platform>(path: &str) -> Result<()>
    where
        Platform: Serialize + DeserializeOwned + 'static,
    {
        let cfg = std::fs::read_to_string(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to find input config file '{path}'"))?;
        let c: MnemosConfig<Platform> = from_toml(&cfg)?;

        let out_dir = std::env::var("OUT_DIR").into_diagnostic()?;
        let mut out = PathBuf::from(out_dir);

        out.push("mnemos-config.postcard");
        let bin_cfg = to_postcard(&c)?;
        let mut f = std::fs::File::create(&out).unwrap();
        f.write_all(&bin_cfg).unwrap();
        println!("cargo:rustc-env=MNEMOS_CONFIG={}", out.display());
        println!("cargo:rerun-if-changed={path}");

        Ok(())
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
macro_rules! load_configuration {
    ($platform: ty) => {{
        const MNEMOS_CONFIG: &[u8] = include_bytes!(env!("MNEMOS_CONFIG"));
        $crate::runtime::from_postcard::<$platform>(MNEMOS_CONFIG)
    }};
}
