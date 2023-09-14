#![no_std]
use core::time::Duration;
use serde::{Deserialize, Serialize};

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
        Mapping::Twi0
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Mapping {
    Twi0,
    Twi1,
    Twi2,
    Twi3,
}

// I2C Puppet

#[derive(Debug, Serialize, Deserialize)]
pub struct I2cPuppetConfiguration {
    #[serde(default)]
    pub enabled: bool,
    pub interrupt_pin: Option<InterruptPin>,
    #[serde(default = "I2cPuppetConfiguration::default_poll_interval")]
    pub poll_interval: Duration,
}

impl I2cPuppetConfiguration {
    const fn default_poll_interval() -> Duration {
        Duration::from_millis(50)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum InterruptPin {
    PB7,
}

// LED service

#[derive(Debug, Serialize, Deserialize)]
pub struct LedBlinkService {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "LedBlinkService::default_led_pin")]
    pub blink_pin: LedBlinkPin,
    #[serde(default = "LedBlinkService::default_blink_interval")]
    pub blink_interval: Duration,
}

impl LedBlinkService {
    const fn default_led_pin() -> LedBlinkPin {
        LedBlinkPin::PC1
    }

    const fn default_blink_interval() -> Duration {
        Duration::from_millis(250)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LedBlinkPin {
    PC1,
    PD18,
}
