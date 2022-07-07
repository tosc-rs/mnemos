use serde::{Serialize, Deserialize};
use super::ByteBoxWire;

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum SerialRequest {
    OpenPort {
        port: u16,
    },
    ProvideReceiveBuffer {
        port: u16,
        buffer: ByteBoxWire
    },
    Flush {
        port: u16,
    },
    SendData {
        port: u16,
        buffer: ByteBoxWire,
        used: usize,
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum SerialResponse {
    OpenPort {
        port: u16,
    },
    ReceiveData {
        port: u16,
        buffer: ByteBoxWire,
        used: usize,
    },
    FlushAck {
        port: u16,
    },
    SendComplete {
        port: u16,
        buffer: ByteBoxWire,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum SerialError {
    Unknown,
}
