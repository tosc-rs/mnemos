use common::{
    syscall::request::SysCallRequest,
    syscall::{
        request::{BlockRequest, GpioMode, GpioRequest, SpiRequest, SystemRequest},
        success::{
            BlockInfo, BlockSuccess, GpioSuccess, SpiSuccess, StoreInfo, SysCallSuccess,
            SystemSuccess, TimeSuccess,
        },
    },
    syscall::{
        request::{SerialRequest, TimeRequest},
        success::SerialSuccess,
        BlockKind,
    },
};
use groundhog::RollingTimer;
use groundhog_nrf52::GlobalRollingTimer;

use crate::alloc::HeapGuard;

pub trait OutputPin: Send {
    fn set_pin(&mut self, is_high: bool);
}

pub trait GpioPin: Send {
    fn set_mode(&mut self, mode: GpioMode) -> Result<(), ()>;
    fn read_pin(&mut self) -> Result<bool, ()>;
    fn set_pin(&mut self, is_high: bool) -> Result<(), ()>;
}

pub trait Spi: Send {
    fn send<'a>(&mut self, csn: u8, speed_khz: u32, data_out: &'a [u8]) -> Result<(), ()>;
    fn transfer<'a>(
        &mut self,
        csn: u8,
        speed_khz: u32,
        data_out: &'a [u8],
        data_in: &'a mut [u8],
    ) -> Result<&'a mut [u8], ()>;
    fn read<'a>(
        &mut self,
        csn: u8,
        speed_khz: u32,
        dummy_char: u8,
        data_in: &'a mut [u8],
    ) -> Result<&'a mut [u8], ()>;
}

pub trait Serial: Send {
    fn register_port(&mut self, port: u16) -> Result<(), ()>;
    fn release_port(&mut self, port: u16) -> Result<(), ()>;
    fn process(&mut self, heap: &mut HeapGuard);

    // On success: The valid received part (<= buf.len()). Can be &[] (if no bytes)
    // On error: TODO
    fn recv<'a>(
        &mut self,
        heap: &mut HeapGuard,
        port: u16,
        buf: &'a mut [u8],
    ) -> Result<&'a mut [u8], ()>;

    // On success: All bytes were sent/enqueued.
    // On error: the portion of bytes that were NOT sent (the remainder). (<= buf.len()).
    // CANNOT be &[].
    fn send<'a>(&mut self, port: u16, buf: &'a [u8]) -> Result<(), &'a [u8]>;
}

pub trait BlockStorage: Send {
    fn block_count(&self) -> u32;
    fn block_size(&self) -> u32;
    fn block_info<'a>(&self, block: u32, name_buf: &'a mut [u8]) -> Result<BlockInfo<'a>, ()>;
    fn block_open(&mut self, block: u32) -> Result<(), ()>;
    fn block_write(&mut self, block: u32, offset: u32, data: &[u8]) -> Result<(), ()>;
    fn block_read<'a>(
        &mut self,
        block: u32,
        offset: u32,
        data: &'a mut [u8],
    ) -> Result<&'a mut [u8], ()>;
    fn block_close(
        &mut self,
        heap: &mut HeapGuard,
        block: u32,
        name: &str,
        len: u32,
        kind: BlockKind,
    ) -> Result<(), ()>;
    unsafe fn block_load_to(
        &mut self,
        block: u32,
        dest: *mut u8,
        max_len: usize,
    ) -> Result<(*const u8, usize), ()>;
}

pub struct Machine {
    pub serial: &'static mut dyn Serial,
    pub block_storage: Option<&'static mut dyn BlockStorage>,
    pub spi: Option<&'static mut dyn Spi>,
    pub gpios: &'static mut [&'static mut dyn GpioPin],
}

impl Machine {
    pub fn handle_syscall<'a>(
        &'a mut self,
        heap: &mut HeapGuard,
        req: SysCallRequest<'a>,
    ) -> Result<SysCallSuccess<'a>, ()> {
        match req {
            SysCallRequest::Time(tr) => {
                let resp = self.handle_time_request(tr)?;
                Ok(SysCallSuccess::Time(resp))
            }
            SysCallRequest::Serial(sr) => {
                let resp = self.handle_serial_request(heap, sr)?;
                Ok(SysCallSuccess::Serial(resp))
            }
            SysCallRequest::BlockStore(bsr) => {
                let resp = self.handle_block_request(heap, bsr)?;
                Ok(SysCallSuccess::BlockStore(resp))
            }
            SysCallRequest::System(sr) => {
                let resp = self.handle_system_request(sr)?;
                Ok(SysCallSuccess::System(resp))
            }
            SysCallRequest::Spi(sr) => {
                let resp = self.handle_spi_request(sr)?;
                Ok(SysCallSuccess::Spi(resp))
            }
            SysCallRequest::Gpio(gr) => {
                let resp = self.handle_gpio_request(gr)?;
                Ok(SysCallSuccess::Gpio(resp))
            }
        }
    }

    fn handle_spi_request<'a>(&mut self, sr: SpiRequest<'a>) -> Result<SpiSuccess<'a>, ()> {
        let spi = self.spi.as_mut().ok_or(())?;

        match sr {
            SpiRequest::Send {
                csn,
                data_out,
                speed_khz,
            } => {
                let buf = unsafe { data_out.to_slice() };
                spi.send(csn, speed_khz, buf)?;
                Ok(SpiSuccess::SendSuccess)
            }
            SpiRequest::Transfer {
                csn,
                data_out,
                data_in,
                speed_khz,
            } => {
                let buf_in = unsafe { data_in.to_slice_mut() };
                let buf_out = unsafe { data_out.to_slice() };
                let buf_in = spi.transfer(csn, speed_khz, buf_out, buf_in)?;
                Ok(SpiSuccess::Transfer {
                    data_in: buf_in.into(),
                })
            }
            SpiRequest::Read {
                csn,
                dummy_byte,
                data_in,
                speed_khz,
            } => {
                let buf_in = unsafe { data_in.to_slice_mut() };
                let buf_in = spi.read(csn, speed_khz, dummy_byte, buf_in)?;
                Ok(SpiSuccess::Read {
                    data_in: buf_in.into(),
                })
            }
        }
    }

    fn handle_gpio_request(&mut self, gr: GpioRequest) -> Result<GpioSuccess, ()> {
        let pin: usize = gr.pin().into();
        let gpio = self.gpios.get_mut(pin).ok_or(())?;

        match gr {
            GpioRequest::SetMode { mode, .. } => {
                gpio.set_mode(mode)?;
                Ok(GpioSuccess::ModeSet)
            }
            GpioRequest::ReadInput { .. } => gpio
                .read_pin()
                .map(|is_high| GpioSuccess::ReadInput { is_high }),
            GpioRequest::WriteOutput { is_high, .. } => {
                gpio.set_pin(is_high)?;
                Ok(GpioSuccess::OutputWritten)
            }
        }
    }

    pub fn handle_system_request(&mut self, req: SystemRequest) -> Result<SystemSuccess, ()> {
        match req {
            SystemRequest::SetBootBlock { block } => {
                crate::MAGIC_BOOT.set(block);
                Ok(SystemSuccess::BootBlockSet)
            }
            SystemRequest::Reset => {
                defmt::println!("Rebooting!");
                let timer = GlobalRollingTimer::default();
                let start = timer.get_ticks();
                while timer.millis_since(start) <= 1000 {}
                nrf52840_hal::pac::SCB::sys_reset();
            }
        }
    }

    pub fn handle_time_request(&mut self, req: TimeRequest) -> Result<TimeSuccess, ()> {
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
        heap: &mut HeapGuard,
        req: SerialRequest<'a>,
    ) -> Result<SerialSuccess<'a>, ()> {
        match req {
            SerialRequest::SerialReceive { port, dest_buf } => {
                let dest_buf = unsafe { dest_buf.to_slice_mut() };
                let used = self.serial.recv(heap, port, dest_buf)?;
                Ok(SerialSuccess::DataReceived {
                    dest_buf: used.into(),
                })
            }
            SerialRequest::SerialSend { port, src_buf } => {
                let src_buf = unsafe { src_buf.to_slice() };
                match self.serial.send(port, src_buf) {
                    Ok(()) => Ok(SerialSuccess::DataSent { remainder: None }),
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
        heap: &mut HeapGuard,
        req: BlockRequest<'a>,
    ) -> Result<BlockSuccess<'a>, ()> {
        // Match early to provide the "null" storage info if we have none.
        let sto: &mut dyn BlockStorage = match (self.block_storage.as_mut(), &req) {
            (None, BlockRequest::StoreInfo) => {
                return Ok(BlockSuccess::StoreInfo(StoreInfo {
                    blocks: 0,
                    capacity: 0,
                }));
            }
            (None, _) => return Err(()),
            (Some(sto), _) => *sto,
        };

        match req {
            BlockRequest::StoreInfo => Ok(BlockSuccess::StoreInfo(StoreInfo {
                blocks: sto.block_count(),
                capacity: sto.block_size(),
            })),
            BlockRequest::BlockInfo {
                block_idx,
                name_buf,
            } => {
                let name_buf = unsafe { name_buf.to_slice_mut() };
                let info = sto.block_info(block_idx, name_buf)?;
                Ok(BlockSuccess::BlockInfo(info))
            }
            BlockRequest::BlockOpen { block_idx } => {
                sto.block_open(block_idx)?;
                Ok(BlockSuccess::BlockOpened)
            }
            BlockRequest::BlockWrite {
                block_idx,
                offset,
                src_buf,
            } => {
                sto.block_write(block_idx, offset, unsafe { src_buf.to_slice() })?;
                Ok(BlockSuccess::BlockWritten)
            }
            BlockRequest::BlockRead {
                block_idx,
                offset,
                dest_buf,
            } => {
                let buf = unsafe { dest_buf.to_slice_mut() };
                let dest = sto.block_read(block_idx, offset, buf)?;
                Ok(BlockSuccess::BlockRead {
                    dest_buf: dest.into(),
                })
            }
            BlockRequest::BlockClose {
                block_idx,
                name,
                len,
                kind,
            } => {
                let name_bytes = unsafe { name.to_slice() };
                let name = core::str::from_utf8(name_bytes).map_err(drop)?;
                sto.block_close(heap, block_idx, name, len, kind)?;
                Ok(BlockSuccess::BlockClosed)
            }
        }
    }
}
