#![cfg_attr(not(any(feature = "use-std", test)), no_std)]

use mnemos_kernel::{DefaultServiceSettings, KernelConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MnemosConfig<Platform> {
    pub kernel_cfg: KernelConfig,
    pub kernel_svc_cfg: DefaultServiceSettings,
    pub platform_cfg: Platform,
}

#[cfg(feature = "use-std")]
pub mod buildtime {
    use std::{io::Write, path::PathBuf};

    use super::*;
    use serde::de::DeserializeOwned;
    use miette::{Result, IntoDiagnostic, Context};

    fn from_toml<Platform>(s: &str) -> Result<MnemosConfig<Platform>>
    where
        Platform: DeserializeOwned + 'static,
    {
        Ok(toml::from_str(s).into_diagnostic()?)
    }

    fn to_postcard<Platform>(mc: &MnemosConfig<Platform>) -> Result<Vec<u8>>
    where
        Platform: Serialize,
    {
        Ok(postcard::to_stdvec(&mc).into_diagnostic()?)
    }

    pub fn render_project<Platform>(path: &str) -> Result<()>
    where
        Platform: Serialize + DeserializeOwned + 'static,
    {
        let cfg = std::fs::read_to_string(path).into_diagnostic()
        .wrap_err_with(|| {
            format!("Failed to find input config file '{path}'")
        })?;
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

pub mod runtime {
    use serde::de::DeserializeOwned;

    use crate::MnemosConfig;

    pub fn from_postcard<Platform>(s: &[u8]) -> Result<MnemosConfig<Platform>, ()>
    where
        Platform: DeserializeOwned + 'static,
    {
        Ok(postcard::from_bytes(s).unwrap())
    }
}

#[macro_export]
macro_rules! load_configuration {
    ($platform: ty) => {
        {
            const MELPO_CFG: &[u8] = include_bytes!(env!("MNEMOS_CONFIG"));
            ::config::runtime::from_postcard::<$platform>(MELPO_CFG)
        }
    };
}
