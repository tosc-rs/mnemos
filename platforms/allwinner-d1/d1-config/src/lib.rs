#![no_std]

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub i2c: I2cConfiguration,
    pub i2c_puppet: I2cPuppetConfiguration,
    pub blink_service: LedBlinkService,
}

// I2C

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

// I2C Puppet

#[derive(Debug, Serialize, Deserialize)]
pub struct I2cPuppetConfiguration {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "I2cPuppetConfiguration::default_interrupt_pin")]
    pub interrupt_pin: InterruptPin,
}

impl I2cPuppetConfiguration {
    const fn default_interrupt_pin() -> InterruptPin {
        InterruptPin::PB7
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum InterruptPin {
    PB7
}

// LED service

#[derive(Debug, Serialize, Deserialize)]
pub struct LedBlinkService {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "LedBlinkService::default_led_pin")]
    pub blink_pin: LedBlinkPin,
}

impl LedBlinkService {
    const fn default_led_pin() -> LedBlinkPin {
        LedBlinkPin::PC1
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum LedBlinkPin {
    PC1,
    PD18,
}
