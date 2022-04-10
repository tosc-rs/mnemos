use common::{
    syscall::request::SysCallRequest,
    syscall::{success::{SysCallSuccess, TimeSuccess, BlockSuccess, BlockInfo}, request::BlockRequest},
    syscall::{
        request::{SerialRequest, TimeRequest},
        success::SerialSuccess,
        BlockKind,
    },
};
use groundhog::RollingTimer;
use groundhog_nrf52::GlobalRollingTimer;

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

pub trait BlockStorage: Send {
    fn block_count(&self) -> u32;
    fn block_size(&self) -> u32;
    fn block_info<'a>(&'a self, block: u32) -> Result<BlockInfo<'a>, ()>;
    fn block_open(&mut self, block: u32) -> Result<(), ()>;
    fn block_write(&mut self, block: u32, offset: u32, data: &[u8]) -> Result<(), ()>;
    fn block_read<'a>(&mut self, block: u32, offset: u32, data: &'a mut [u8]) -> Result<&'a mut [u8], ()>;
    fn block_close(&mut self, block: u32, name: &str, len: u32, kind: BlockKind) -> Result<(), ()>;
}

pub struct Machine {
    pub serial: &'static mut dyn Serial,
    pub block_storage: Option<&'static mut dyn BlockStorage>,
}

impl Machine {
    pub fn handle_syscall<'a>(
        &'a mut self,
        req: SysCallRequest<'a>,
    ) -> Result<SysCallSuccess<'a>, ()> {
        match req {
            SysCallRequest::Time(tr) => {
                let resp = self.handle_time_request(tr)?;
                Ok(SysCallSuccess::Time(resp))
            }
            SysCallRequest::Serial(sr) => {
                let resp = self.handle_serial_request(sr)?;
                Ok(SysCallSuccess::Serial(resp))
            }
            SysCallRequest::BlockStore(bsr) => {
                let resp = self.handle_block_request(bsr)?;
                Ok(SysCallSuccess::BlockStore(resp))
            },
        }
    }

    pub fn handle_time_request(
        &mut self,
        req: TimeRequest,
    ) -> Result<TimeSuccess, ()> {
        let TimeRequest::SleepMicros { us } = req;

        let timer = GlobalRollingTimer::default();
        let mut ttl_us = us;
        let orig_start = timer.get_ticks();

        // Just in case the user asks for something REALLY close to a rollover (e.g. u32::MAX),
        // don't delay for more than half of the range of the timer.
        while ttl_us != 0 {
            let start = timer.get_ticks();
            let to_wait = ttl_us.min(u32::MAX / 2);
            while timer.micros_since(start) <= to_wait {}
            ttl_us = ttl_us.saturating_sub(to_wait)
        }

        Ok(TimeSuccess::SleptMicros {
            us: timer.micros_since(orig_start).min(us),
        })
    }

    pub fn handle_serial_request<'a>(
        &mut self,
        req: SerialRequest<'a>
    ) -> Result<SerialSuccess<'a>, ()> {
        match req {
            SerialRequest::SerialReceive { port, dest_buf } => {
                let dest_buf = unsafe { dest_buf.to_slice_mut() };
                let used = self.serial.recv(port, dest_buf)?;
                Ok(SerialSuccess::DataReceived {
                    dest_buf: used.into(),
                })
            }
            SerialRequest::SerialSend { port, src_buf } => {
                let src_buf = unsafe { src_buf.to_slice() };
                match self.serial.send(port, src_buf) {
                    Ok(()) => Ok(SerialSuccess::DataSent {
                        remainder: None,
                    }),
                    Err(rem) => Ok(SerialSuccess::DataSent {
                        remainder: Some(rem.into()),
                    }),
                }
            }
            SerialRequest::SerialOpenPort { port } => {
                self.serial.register_port(port)?;
                Ok(SerialSuccess::PortOpened)
            }
        }
    }

    pub fn handle_block_request<'a>(
        &'a mut self,
        req: BlockRequest<'a>,
    ) -> Result<BlockSuccess<'a>, ()> {
        // Match early to provide the "null" storage info if we have none.
        let sto: &mut dyn BlockStorage = match (self.block_storage.as_mut(), &req) {
            (None, BlockRequest::StoreInfo) => {
                return Ok(BlockSuccess::StoreInfo {
                    blocks: 0,
                    capacity: 0,
                });
            },
            (None, _) => return Err(()),
            (Some(sto), _) => *sto,
        };

        match req {
            BlockRequest::StoreInfo => {
                Ok(BlockSuccess::StoreInfo {
                    blocks: sto.block_count(),
                    capacity: sto.block_size(),
                })
            },
            BlockRequest::BlockInfo { block_idx } => {
                let info = sto.block_info(block_idx)?;
                Ok(BlockSuccess::BlockInfo(info))
            },
            BlockRequest::BlockOpen { block_idx } => {
                sto.block_open(block_idx)?;
                Ok(BlockSuccess::BlockOpened)
            },
            BlockRequest::BlockWrite { block_idx, offset, src_buf } => {
                sto.block_write(block_idx, offset, unsafe { src_buf.to_slice() })?;
                Ok(BlockSuccess::BlockWritten)
            },
            BlockRequest::BlockRead { block_idx, offset, dest_buf } => {
                let buf = unsafe { dest_buf.to_slice_mut() };
                let dest = sto.block_read(block_idx, offset, buf)?;
                Ok(BlockSuccess::BlockRead { dest_buf: dest.into() })
            },
            BlockRequest::BlockClose { block_idx, name, len, kind } => {
                let name_bytes = unsafe { name.to_slice() };
                let name = core::str::from_utf8(name_bytes).map_err(drop)?;
                sto.block_close(block_idx, name, len, kind)?;
                Ok(BlockSuccess::BlockClosed)
            },
        }
    }
}
