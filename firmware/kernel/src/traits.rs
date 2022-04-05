use crate::syscall::{SysCallRequest, SysCallSuccess};

pub trait Serial: Send {
    fn register_port(&mut self, port: u16) -> Result<(), ()>;
    fn release_port(&mut self, port: u16) -> Result<(), ()>;
    fn process(&mut self);

    // On success: The valid received part (<= buf.len()). Can be &[] (if no bytes)
    // On error: TODO
    fn recv<'a>(&mut self, port: u16, buf: &'a mut [u8]) -> Result<&'a mut [u8], ()>;

    // On success: All bytes were sent/enqueued.
    // On error: the portion of bytes that were NOT sent (the remainder). (<= buf.len()).
    // CANNOT be &[].
    fn send<'a>(&mut self, port: u16, buf: &'a [u8]) -> Result<(), &'a [u8]>;
}

// pub trait SendSerial: Serial + Send {}

pub struct Machine {
    pub serial: &'static mut dyn Serial,
    // TODO: port router?
    // TODO: flash manager?
}

impl Machine {
    pub fn handle_syscall<'a>(&mut self, req: SysCallRequest<'a>) -> Result<SysCallSuccess<'a>, ()> {
        match req {
            SysCallRequest::SerialReceive { port, dest_buf } => {
                let dest_buf = unsafe { dest_buf.to_slice_mut() };
                let used = self.serial.recv(port, dest_buf)?;
                Ok(SysCallSuccess::DataReceived { dest_buf: used.into() })
            },
            SysCallRequest::SerialSend { port, src_buf } => {
                let src_buf = unsafe { src_buf.to_slice() };
                match self.serial.send(port, src_buf) {
                    Ok(()) => {
                        Ok(SysCallSuccess::DataSent { remainder: None })
                    }
                    Err(rem) => {
                        Ok(SysCallSuccess::DataSent { remainder: Some(rem.into()) })
                    },
                }
            },
            SysCallRequest::SerialOpenPort { port } => {
                self.serial.register_port(port)?;
                Ok(SysCallSuccess::PortOpened)
            },
        }
    }
}
