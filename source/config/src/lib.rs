#![cfg_attr(not(any(feature = "use-std", test)), no_std)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MnemosConfig<KC, KSC, PC> {
    pub kernel_cfg: KC,
    pub kernel_svc_cfg: KSC,
    pub platform_cfg: PC,
}

#[cfg(feature = "use-std")]
pub mod buildtime {
    use serde::de::DeserializeOwned;

    use super::*;

    pub fn from_toml<KC, KSC, PC>(s: &str) -> Result<MnemosConfig<KC, KSC, PC>, ()>
    where
        KC: DeserializeOwned + 'static,
        KSC: DeserializeOwned + 'static,
        PC: DeserializeOwned + 'static,
    {
        Ok(toml::from_str(s).unwrap())
    }

    pub fn to_postcard<KC, KSC, PC>(mc: &MnemosConfig<KC, KSC, PC>) -> Result<Vec<u8>, ()>
    where
        KC: Serialize,
        KSC: Serialize,
        PC: Serialize,
    {
        postcard::to_stdvec(&mc).map_err(drop)
    }
}

pub mod runtime {
    use serde::de::DeserializeOwned;

    use crate::MnemosConfig;

    pub fn from_postcard<KC, KSC, PC>(s: &[u8]) -> Result<MnemosConfig<KC, KSC, PC>, ()>
    where
        KC: DeserializeOwned + 'static,
        KSC: DeserializeOwned + 'static,
        PC: DeserializeOwned + 'static,
    {
        Ok(postcard::from_bytes(s).unwrap())
    }
}
