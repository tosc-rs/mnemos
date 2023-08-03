#![no_std]

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub i2c: I2cConfiguration,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct I2cConfiguration {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "I2cConfiguration::default_mapping")]
    pub mapping: Mapping,
}

impl I2cConfiguration {
    const fn default_mapping() -> Mapping {
        Mapping::LicheeRvTwi0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Mapping {
    LicheeRvTwi0,
    MangoPiTwi2,
}
