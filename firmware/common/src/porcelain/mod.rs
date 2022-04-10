// TODO: To be honest, these are a bit more "plumbing" than "porcelain".
//
// The "real" porcelain should feel more like the stdlib, these are more
// safe wrappers of -sys functions. At some point move these functions
// to a "plumbing" module, and add a higher level "porcelain" level

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
        success::{BlockSuccess, StoreInfo, BlockStatus}, BlockKind,
    };

    // TODO: I should probably come up with a policy for how to handle types that
    // remove the SysCallSlice* types. For example, in other porcelain calls, I just
    // return slices (instead of SCS* types). This is sort of type duplication, BUT it's
    // probably also good practice NOT to expose any of the syscall "wire types" to the
    // end user.
    //
    // Something to think about, at least.
    pub struct BlockInfoStr<'a>{
        pub length: u32,
        pub capacity: u32,
        pub kind: BlockKind,
        pub status: BlockStatus,
        pub name: Option<&'a str>,
    }

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

    pub fn block_info<'a>(block: u32, name_buf: &'a mut [u8]) -> Result<BlockInfoStr<'a>, ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockInfo {
            block_idx: block,
            name_buf: name_buf.into()
        });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockInfo(bi) = resp {
            Ok(BlockInfoStr {
                length: bi.length,
                capacity: bi.capacity,
                kind: bi.kind,
                status: bi.status,
                name: bi.name.and_then(|scs| {
                    let bytes = unsafe { scs.to_slice() };
                    // TODO: this *probably* could be unchecked, but for now
                    // just report no name on an invalid decode
                    core::str::from_utf8(bytes).ok()
                }),
            })
        } else {
            Err(())
        }
    }

    pub fn block_open(block: u32) -> Result<(), ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockOpen { block_idx: block });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockOpened = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn block_read<'a>(
        block: u32,
        offset: u32,
        dest_buf: &'a mut [u8]
    ) -> Result<&'a mut [u8], ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockRead {
            block_idx: block,
            offset,
            dest_buf: dest_buf.into(),
        });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockRead { dest_buf } = resp {
            Ok(unsafe { dest_buf.to_slice_mut() })
        } else {
            Err(())
        }
    }

    pub fn block_write(block: u32, offset: u32, bytes: &[u8]) -> Result<(), ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockWrite {
            block_idx: block,
            offset,
            src_buf: bytes.into(),
        });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockWritten = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn block_close(block: u32, name: &str, len: u32, kind: BlockKind) -> Result<(), ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockClose {
            block_idx: block,
            name: name.as_bytes().into(),
            len,
            kind,
        });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockClosed = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    // TODO: I should probably have some kind of `Block` type that does closing and stuff
    // in a typical `File` kind of way.
}
