use abi::syscall::{UserRequestBody, serial::{SerialRequest, SerialError, SerialResponse}, KernelResponseBody};
use crate::executor::mailbox::MAILBOX;

pub struct SerialPort {
    port: u16,
}

impl SerialPort {
    pub async fn open(req_port: u16) -> Result<Self, SerialError> {
        let msg = UserRequestBody::Serial(SerialRequest::OpenPort { port: req_port });
        if let Ok(KernelResponseBody::Serial(sr)) = MAILBOX.request(msg).await {
            if let SerialResponse::OpenPort { port } = sr? {
                if port == req_port {
                    return Ok(SerialPort { port });
                }
            }
        }
        Err(SerialError::Unknown)
    }
}
