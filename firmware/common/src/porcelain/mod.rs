use crate::syscall::{
    request::SysCallRequest,
    success::SysCallSuccess,
    try_syscall,
};

pub mod serial {
    use super::*;
    use crate::syscall::request::SerialRequest;
    use crate::syscall::success::SerialSuccess;

    fn success_filter(succ: SysCallSuccess) -> Result<SerialSuccess, ()> {
        if let SysCallSuccess::Serial(s) = succ {
            Ok(s)
        } else {
            Err(())
        }
    }

    pub fn open_port(port: u16) -> Result<(), ()> {
        let req = SysCallRequest::Serial(SerialRequest::SerialOpenPort { port });

        let res = success_filter(try_syscall(req)?)?;

        if let SerialSuccess::PortOpened = res {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn read_port(port: u16, data: &mut [u8]) -> Result<&mut [u8], ()> {
        let req = SysCallRequest::Serial(SerialRequest::SerialReceive {
            port,
            dest_buf: data.as_mut().into(),
        });

        let resp = success_filter(try_syscall(req)?)?;

        if let SerialSuccess::DataReceived { dest_buf } = resp {
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
        let req = SysCallRequest::Serial(SerialRequest::SerialSend {
            port,
            src_buf: data.into(),
        });

        let resp = success_filter(try_syscall(req)?)?;

        match resp {
            SerialSuccess::DataSent { remainder: Some(rem) } => {
                let remlen = rem.len as usize;
                let datlen = data.len();

                if remlen <= datlen {
                    Ok(Some(&data[(datlen - remlen)..]))
                } else {
                    // Unexpected!
                    Err(())
                }
            }
            SerialSuccess::DataSent { remainder: None } => {
                Ok(None)
            }
            _ => Err(()),
        }
    }
}

pub mod time {
    use crate::syscall::{success::TimeSuccess, request::TimeRequest};

    use super::*;

    fn success_filter(succ: SysCallSuccess) -> Result<TimeSuccess, ()> {
        if let SysCallSuccess::Time(s) = succ {
            Ok(s)
        } else {
            Err(())
        }
    }

    pub fn sleep_micros(us: u32) -> Result<u32, ()> {
        let req = SysCallRequest::Time(TimeRequest::SleepMicros { us });
        let resp = success_filter(try_syscall(req)?)?;

        let TimeSuccess::SleptMicros { us } = resp;
        Ok(us)
    }
}

pub mod block_storage {
    use super::*;
    use crate::syscall::{
        request::BlockRequest,
        success::{BlockSuccess, StoreInfo, BlockInfo},
    };

    pub fn success_filter(succ: SysCallSuccess) -> Result<BlockSuccess, ()> {
        if let SysCallSuccess::BlockStore(bsr) = succ {
            Ok(bsr)
        } else {
            Err(())
        }
    }

    pub fn store_info() -> Result<StoreInfo, ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::StoreInfo);
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::StoreInfo(si) = resp {
            Ok(si)
        } else {
            Err(())
        }
    }

    pub fn block_info<'a>(block: u32, name_buf: &'a mut [u8]) -> Result<BlockInfo<'a>, ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockInfo {
            block_idx: block,
            name_buf: name_buf.into()
        });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockInfo(bi) = resp {
            Ok(bi)
        } else {
            Err(())
        }
    }

}
