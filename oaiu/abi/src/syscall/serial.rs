use serde::{Serialize, Deserialize};
use super::ByteBoxWire;

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum SerialRequest {
    OpenPort {
        port: u16,
    },
    ProvideReceiveBuffer {
        buffer: ByteBoxWire
    },
    Flush,
    SendData {
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
        buffer: ByteBoxWire,
        used: usize,
    },
    FlushAck,
    SendComplete {
        buffer: ByteBoxWire,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum SerialError {
    Unknown,
}
