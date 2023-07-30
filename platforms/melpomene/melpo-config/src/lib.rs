use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct PlatformConfig {
    lol: u8,
}
