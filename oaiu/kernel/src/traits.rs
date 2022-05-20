use core::alloc::Layout;

use common::{
    syscall::request::SysCallRequest,
    syscall::{
        request::{BlockRequest, GpioMode, GpioRequest, SystemRequest, PcmSinkRequest},
        success::{
            BlockInfo, BlockSuccess, GpioSuccess, StoreInfo, SysCallSuccess,
            SystemSuccess, TimeSuccess, PcmSinkSuccess,
        },
        future::{SysCallFuture, SCFutureKind, FutureBox},
    },
    syscall::{
        request::{SerialRequest, TimeRequest},
        success::SerialSuccess,
        BlockKind,
    },
};
use cortex_m::peripheral::SCB;
use groundhog::RollingTimer;
use groundhog_nrf52::GlobalRollingTimer;

use crate::{alloc::{HeapGuard, HeapArray}, future_box::FutureBoxExHdl, DriverCommand, DRIVER_QUEUE};

pub trait RandFill: Send {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), ()>;
}

pub trait OutputPin: Send {
    fn set_pin(&mut self, is_high: bool);
}

pub trait GpioPin: Send {
    fn set_mode(&mut self, mode: GpioMode) -> Result<(), ()>;
    fn read_pin(&mut self) -> Result<bool, ()>;
    fn set_pin(&mut self, is_high: bool) -> Result<(), ()>;
}

pub struct SpiTransaction {
    pub kind: SpiTransactionKind,
    pub data: HeapArray<u8>,
    pub hdl: SpiHandle,
    pub speed_khz: u32,
}

pub enum SpiTransactionKind {
    Send
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub struct SpiHandle {
    pub(crate) idx: u8,
}

pub trait SpimNode: Send {
    /// Set the node active. This should set any chip selects
    /// necessary, and enable any additional behavior, such as an interrupt
    /// or other event that would stop a transfer early, such as a "BUSY"
    /// pin going high.
    fn set_active(&mut self);

    /// Set the node inactive. This should clear any chip selects necessary,
    /// and disable any additional behavior as described in `set_active`.
    fn set_inactive(&mut self);

    /// This should be used to tell the Spim device whether a new transfer
    /// should be started. If your device is always ready, you do not need
    /// to implement this method. If your device has some kind of "BUSY"
    /// pin or similar that should be respected, you should override this
    /// method with necessary handling.
    fn is_ready(&mut self) -> bool {
        true
    }
}

pub trait PcmSink: Send {
    // TODO: Always assumes stereo 16-bit signed PCM audio, at 44100Hz sample rate
    fn enable(&mut self, heap: &mut HeapGuard, spi: &mut dyn Spi) -> Result<(), ()>;
    fn disable(&mut self, heap: &mut HeapGuard, spi: &mut dyn Spi) -> Result<(), ()>;
    // TODO: This leaks the fact that we only support a SPI device... How do handle
    // going from Future<PcmSamples> to Future<SpiTransaction>?
    fn allocate_stereo_samples(&mut self, heap: &mut HeapGuard, spi: &mut dyn Spi, count: usize) -> Option<FutureBoxExHdl<SpiTransaction>>;
}

pub trait Spi: Send {
    fn register_handle(
        &mut self,
        node: &'static mut dyn SpimNode,
    ) -> Result<SpiHandle, &'static mut dyn SpimNode>;

    fn alloc_transaction(
        &mut self,
        heap: &mut HeapGuard,
        kind: SpiTransactionKind,
        hdl: SpiHandle,
        speed_khz: u32,
        count: usize,
    ) -> Option<FutureBoxExHdl<SpiTransaction>>;
    fn start_send(&mut self);
    fn end_send(&mut self);
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
    pub pcm: Option<&'static mut dyn PcmSink>,
    pub rand: Option<&'static mut dyn RandFill>,
}

impl Machine {
    pub fn handle_syscall<'a>(
        &mut self,
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
                let resp = self.handle_system_request(heap, sr)?;
                Ok(SysCallSuccess::System(resp))
            }
            SysCallRequest::Gpio(gr) => {
                let resp = self.handle_gpio_request(gr)?;
                Ok(SysCallSuccess::Gpio(resp))
            }
            SysCallRequest::PcmSink(psr) => {
                let resp = self.handle_pcmsink_request(heap, psr)?;
                Ok(SysCallSuccess::PcmSink(resp))
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

    pub fn handle_pcmsink_request(
        &mut self,
        heap: &mut HeapGuard,
        req: PcmSinkRequest,
    ) -> Result<PcmSinkSuccess, ()> {
        let pcm = self.pcm.as_mut().ok_or(())?;
        let spi = self.spi.as_mut().ok_or(())?;

        match req {
            PcmSinkRequest::Enable => {
                // defmt::println!("[PCM] enable");
                pcm.enable(heap, &mut **spi)?;
                Ok(PcmSinkSuccess::Enabled)
            },
            PcmSinkRequest::Disable => {
                pcm.disable(heap, &mut **spi)?;
                Ok(PcmSinkSuccess::Disabled)
            },
            PcmSinkRequest::AllocateSampleBuffer { count } => {
                let fbeh: FutureBoxExHdl<SpiTransaction> = pcm
                    .allocate_stereo_samples(heap, &mut **spi, count as usize).ok_or(())?;
                let payload_layout = Layout::new::<FutureBoxExHdl<SpiTransaction>>();

                let fb: *mut FutureBox<_> = fbeh.fb;

                // defmt::println!("fb: {=u32:08X}", fb as usize as u32);
                let fut = SysCallFuture {
                    ptr_fb: fb as usize as u32,
                    kind: SCFutureKind::Bytes { ptr: fbeh.data.as_ptr() as u32, len: fbeh.data.len() as u32 },
                    is_exclusive: true,
                    payload_size: payload_layout.size() as u32,
                    payload_align: payload_layout.align() as u32,
                };

                // defmt::println!("[PCM] alloc'd {=u32}", count);
                // defmt::println!("[PCM] ref cnt {=u8}", unsafe { (&*fbeh.fb).refcnt.load(Ordering::SeqCst) });

                // DON'T decrement the refcount - it still exists, it's just being sent to userspace
                core::mem::forget(fbeh);
                Ok(PcmSinkSuccess::SampleBuffer { fut })
            },
        }
    }

    pub fn handle_system_request<'a>(&mut self, _heap: &mut HeapGuard, req: SystemRequest<'a>) -> Result<SystemSuccess<'a>, ()> {
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
            SystemRequest::FreeFutureBox { .. } => {
                // defmt::println!("Freeing FB");
                // let fb_ptr: *mut FutureBox<()> = fb_ptr as usize as *const FutureBox<()> as *mut FutureBox<()>;

                // {
                //     let fb_ref = unsafe { &*fb_ptr };
                //     let payload_layout = Layout::from_size_align(
                //         payload_size as usize,
                //         payload_align as usize,
                //     ).map_err(drop)?;

                //     if let Some(ptr) = NonNull::new(fb_ref.payload.load(Ordering::SeqCst).cast::<u8>()) {
                //         unsafe {
                //             heap.free_raw(ptr, payload_layout);
                //         }
                //         fb_ref.payload.store(null_mut(), Ordering::SeqCst);
                //     }
                // }
                // unsafe {
                //     let fb_layout = Layout::new::<FutureBox<()>>();
                //     let ptr = NonNull::new(fb_ptr.cast()).ok_or(())?;
                //     heap.free_raw(ptr, fb_layout);
                // }

                panic!("Freeing from userspace is currently broken, don't do that!");
                // Ok(SystemSuccess::Freed)
            },
            SystemRequest::Panic => {
                defmt::println!("Application panicked!");
                Err(())
            }
            SystemRequest::RandFill { dest } => {
                let rng = self.rand.as_mut().ok_or(())?;
                let buf = unsafe { dest.to_slice_mut() };
                rng.fill(buf)?;
                Ok(SystemSuccess::RandFilled {
                    dest: buf.into()
                })
            },
        }
    }

    pub fn handle_time_request(&mut self, req: TimeRequest) -> Result<TimeSuccess, ()> {
        let TimeRequest::SleepMicros { us } = req;

        DRIVER_QUEUE.enqueue(DriverCommand::SleepMicros(us)).ok();
        SCB::set_pendsv();

        Ok(TimeSuccess::SleptMicros {
            us,
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
        &mut self,
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
