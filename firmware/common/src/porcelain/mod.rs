use crate::{SysCallRequest, SysCallSuccess, try_syscall};

pub mod serial {

    use super::*;

    pub fn open_port(port: u16) -> Result<(), ()> {
        let req = SysCallRequest::SerialOpenPort { port };

        if let SysCallSuccess::PortOpened = try_syscall(req)? {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn read_port(port: u16, data: &mut [u8]) -> Result<&mut [u8], ()> {
        let req = SysCallRequest::SerialReceive {
            port,
            dest_buf: data.as_mut().into(),
        };

        let resp = try_syscall(req)?;

        if let SysCallSuccess::DataReceived { dest_buf } = resp {
            let dblen = dest_buf.len as usize;

            if dblen <= data.len() {
                Ok(&mut data[..dblen])
            } else {
                Err(())
            }
        } else {
            // Unexpected syscall response!
            Err(())
        }
    }

    pub fn write_port(port: u16, data: &[u8]) -> Result<Option<&[u8]>, ()> {
        let req = SysCallRequest::SerialSend {
            port,
            src_buf: data.into(),
        };

        let resp = try_syscall(req)?;

        match resp {
            SysCallSuccess::DataSent { remainder: Some(rem) } => {
                let remlen = rem.len as usize;
                let datlen = data.len();

                if remlen <= datlen {
                    Ok(Some(&data[(datlen - remlen)..]))
                } else {
                    // Unexpected!
                    Err(())
                }
            }
            SysCallSuccess::DataSent { remainder: None } => {
                Ok(None)
            }
            _ => Err(()),
        }
    }
}

pub mod time {
    use super::*;

    pub fn sleep_micros(us: u32) -> Result<u32, ()> {
        let req = SysCallRequest::SleepMicros { us };
        let resp = try_syscall(req)?;
        if let SysCallSuccess::SleptMicros { us } = resp {
            Ok(us)
        } else {
            Err(())
        }
    }
}
