//! Higher Level System Call Functionality

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

pub mod gpio {
    use super::*;
    use crate::syscall::request::{GpioRequest, GpioMode};
    use crate::syscall::success::GpioSuccess;

    fn success_filter(succ: SysCallSuccess) -> Result<GpioSuccess, ()> {
        if let SysCallSuccess::Gpio(s) = succ {
            Ok(s)
        } else {
            Err(())
        }
    }

    pub fn set_mode(pin: u8, mode: GpioMode) -> Result<(), ()> {
        let req = SysCallRequest::Gpio(GpioRequest::SetMode { pin, mode });
        let res = success_filter(try_syscall(req)?)?;

        if let GpioSuccess::ModeSet = res {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn write_output(pin: u8, is_high: bool) -> Result<(), ()> {
        let req = SysCallRequest::Gpio(GpioRequest::WriteOutput { pin, is_high });
        let res = success_filter(try_syscall(req)?)?;

        if let GpioSuccess::OutputWritten = res {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn read_input(pin: u8) -> Result<bool, ()> {
        let req = SysCallRequest::Gpio(GpioRequest::ReadInput { pin });
        let res = success_filter(try_syscall(req)?)?;

        if let GpioSuccess::ReadInput { is_high } = res {
            Ok(is_high)
        } else {
            Err(())
        }
    }
}

/// Capabilities related to Virtual Serial Ports
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

    /// Open a given virtual serial port
    ///
    /// It is not ever necessary to open Port 0, which is opened by default.
    pub fn open_port(port: u16) -> Result<(), ()> {
        let req = SysCallRequest::Serial(SerialRequest::SerialOpenPort { port });

        let res = success_filter(try_syscall(req)?)?;

        if let SerialSuccess::PortOpened = res {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Attempt to read data from a virtual serial port
    ///
    /// On success, the portion read into the `data` buffer is returned.
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

    /// Attempt to write data to a virtual serial port
    ///
    /// On success, the unsent "remainder" portion, if any, is returned. If all
    /// data was sent, `Ok(None)` is returned.
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

/// Capabilities related to time
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

/// Capabilities related to system control
pub mod system {
    use core::sync::atomic::{compiler_fence, Ordering};

    use crate::syscall::{success::SystemSuccess, request::SystemRequest, future::FutureBox};

    use super::*;

    fn success_filter(succ: SysCallSuccess) -> Result<SystemSuccess, ()> {
        if let SysCallSuccess::System(s) = succ {
            Ok(s)
        } else {
            Err(())
        }
    }

    pub fn panic() -> ! {
        let req = SysCallRequest::System(SystemRequest::Panic);
        let _ = try_syscall(req);
        loop {
            compiler_fence(Ordering::SeqCst);
        }
    }

    /// Set a given block index of the block storage device to be booted from
    ///
    /// The block must be non-empty, and contain a valid User Application image.
    pub fn set_boot_block(block: u32) -> Result<(), ()> {
        let req = SysCallRequest::System(SystemRequest::SetBootBlock { block });
        let resp = success_filter(try_syscall(req)?)?;
        if let SystemSuccess::BootBlockSet = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    pub unsafe fn free_future_box(
        fb_ptr: *mut FutureBox<()>,
        payload_size: usize,
        payload_align: usize,
    ) -> Result<(), ()> {
        let req = SysCallRequest::System(SystemRequest::FreeFutureBox {
            fb_ptr: fb_ptr as usize as u32,
            payload_size: payload_size as u32,
            payload_align: payload_align as u32,
        });
        let resp = success_filter(try_syscall(req)?)?;
        if let SystemSuccess::BootBlockSet = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Immediately reboot the system
    ///
    /// If a block index has been set with `set_boot_block()`, then that image
    /// will be booted into on the next boot.
    pub fn reset() -> Result<(), ()> {
        let req = SysCallRequest::System(SystemRequest::Reset);
        let _resp = success_filter(try_syscall(req)?)?;

        // We'll never get here...
        Ok(())
    }
}

pub mod pcm_sink {
    use core::{sync::atomic::Ordering, ops::{Deref, DerefMut}};

    use super::*;
    use crate::syscall::{
        request::PcmSinkRequest,
        success::PcmSinkSuccess, future::{FutureBox, SysCallFuture, SCFutureKind, status},
    };

    fn success_filter(succ: SysCallSuccess) -> Result<PcmSinkSuccess, ()> {
        if let SysCallSuccess::PcmSink(bsr) = succ {
            Ok(bsr)
        } else {
            Err(())
        }
    }

    pub fn enable() -> Result<(), ()> {
        let req = SysCallRequest::PcmSink(PcmSinkRequest::Enable);
        let resp = success_filter(try_syscall(req)?)?;

        if let PcmSinkSuccess::Enabled = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn disable() -> Result<(), ()> {
        let req = SysCallRequest::PcmSink(PcmSinkRequest::Disable);
        let resp = success_filter(try_syscall(req)?)?;

        if let PcmSinkSuccess::Disabled = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    // #[derive(Serialize, Deserialize)]
    // pub struct SysCallFuture {
    //     // Pointer to the future box
    //     pub ptr_fb: u32,
    //     pub kind: SCFutureKind,
    //     pub is_exclusive: bool,
    //     pub payload_size: u32,
    //     pub payload_align: u32,
    // }

    #[derive(Debug)]
    pub struct SampleBufExFut {
        fb_ptr: *mut FutureBox<()>,
        samples_ptr: *mut u8,
        samples_len: usize,
        payload_size: usize,
        payload_align: usize,
    }

    impl Deref for SampleBufExFut {
        type Target = [u8];

        fn deref(&self) -> &Self::Target {
            unsafe { core::slice::from_raw_parts(self.samples_ptr, self.samples_len) }
        }
    }

    impl DerefMut for SampleBufExFut {
        fn deref_mut(&mut self) -> &mut Self::Target {
            unsafe { core::slice::from_raw_parts_mut(self.samples_ptr, self.samples_len) }
        }
    }

    impl Drop for SampleBufExFut {
        fn drop(&mut self) {
            let drop_fb = {
                let fb = unsafe { &*self.fb_ptr };
                let pre_refs = fb.refcnt.fetch_sub(1, Ordering::SeqCst);

                // TODO(AJM): I don't think we should ever just "drop" an exclusive handle
                // For now, always mark the state as ERROR
                fb.status.store(status::ERROR, Ordering::SeqCst);

                // Release our exlusive status
                fb.ex_taken.store(false, Ordering::SeqCst);
                debug_assert!(pre_refs != 0);
                pre_refs <= 1
            };

            // Split off, to avoid reference to self.fb being live
            // SAFETY: This arm only executes if we were the LAST handle to know
            // about this futurebox.
            if drop_fb {
                unsafe {
                    super::system::free_future_box(
                        self.fb_ptr,
                        self.payload_size,
                        self.payload_align,
                    ).unwrap();
                }
            }
        }
    }

    impl SampleBufExFut {
        pub fn send(self) -> SampleBufCompletionFut {
            let fb = unsafe { &*self.fb_ptr };
            fb.status.store(status::KERNEL_ACCESS, Ordering::SeqCst);
            fb.ex_taken.store(false, Ordering::SeqCst);

            struct StdOut;
            use core::fmt::Write;
            impl Write for StdOut {
                fn write_str(&mut self, s: &str) -> core::fmt::Result {
                    super::serial::write_port(0, s.as_bytes()).ok();
                    Ok(())
                }
            }

            writeln!(&mut StdOut, "send: {:08X?}", fb).ok();

            let ret = SampleBufCompletionFut {
                fb_ptr: self.fb_ptr,
                payload_size: self.payload_size,
                payload_align: self.payload_align,
            };

            core::mem::forget(self);

            ret
        }
    }

    pub struct SampleBufCompletionFut {
        fb_ptr: *mut FutureBox<()>,
        payload_size: usize,
        payload_align: usize,
    }

    impl Drop for SampleBufCompletionFut {
        fn drop(&mut self) {
            let drop_fb = {
                let fb = unsafe { &*self.fb_ptr };
                let pre_refs = fb.refcnt.fetch_sub(1, Ordering::SeqCst);

                // Release our exlusive status
                fb.ex_taken.store(false, Ordering::SeqCst);
                debug_assert!(pre_refs != 0);
                pre_refs <= 1
            };

            // Split off, to avoid reference to self.fb being live
            // SAFETY: This arm only executes if we were the LAST handle to know
            // about this futurebox.
            if drop_fb {
                unsafe {
                    super::system::free_future_box(
                        self.fb_ptr,
                        self.payload_size,
                        self.payload_align,
                    ).unwrap();
                }
            }
        }
    }

    impl SampleBufCompletionFut {
        // TODO: Keep in sync with future_box::FutureBoxPendHdl!
        pub fn is_complete(&self) -> Result<bool, ()> {
            let fb = unsafe { &*self.fb_ptr };
            match fb.status.load(Ordering::SeqCst) {
                status::COMPLETED => Ok(true),
                status::ERROR => Err(()),
                _ => Ok(false),
            }
        }
    }

    pub fn alloc_samples(count: usize) -> Result<SampleBufExFut, ()> {
        let req = SysCallRequest::PcmSink(PcmSinkRequest::AllocateSampleBuffer { count: count as u32 });
        let resp = success_filter(try_syscall(req)?)?;

        if let PcmSinkSuccess::SampleBuffer { fut } = resp {
            if let SysCallFuture {
                ptr_fb,
                kind: SCFutureKind::Bytes { ptr, len },
                is_exclusive: true,
                payload_size,
                payload_align
            } = fut {

                struct StdOut;
                use core::fmt::Write;
                impl Write for StdOut {
                    fn write_str(&mut self, s: &str) -> core::fmt::Result {
                        super::serial::write_port(0, s.as_bytes()).ok();
                        Ok(())
                    }
                }

                writeln!(&mut StdOut, "{:08X?}", fut).ok();
                let fub_ptr = ptr_fb as usize as *const FutureBox<()> as *mut FutureBox<()>;
                unsafe {
                    writeln!(&mut StdOut, "{:08X?}", &*fub_ptr).ok();
                }
                let sbef = SampleBufExFut {
                    fb_ptr: fub_ptr,
                    payload_size: payload_size as usize,
                    payload_align: payload_align as usize,
                    samples_ptr: ptr as usize as *const u8 as *mut u8,
                    samples_len: len as usize,
                };

                writeln!(&mut StdOut, "{:08X?}", sbef).ok();

                Ok(sbef)
            } else {
                // uh oh
                loop {
                    super::time::sleep_micros(1_000_000).ok();
                }
                panic!("Got a bad future!");
            }
        } else {
            Err(())
        }
    }
}

/// Capabilities related to the Block Storage Device, currently
/// the external QSPI flash.
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
    /// A type containing information about a single block of a Block Storage Device.
    pub struct BlockInfoStr<'a>{
        /// The used length (in bytes) of the given block
        pub length: u32,

        /// The capacity (in bytes) of the given block
        pub capacity: u32,

        /// The "kind" of the given block
        pub kind: BlockKind,

        /// The current status of the given block
        pub status: BlockStatus,

        /// The file name of the given block, if any
        pub name: Option<&'a str>,
    }

    fn success_filter(succ: SysCallSuccess) -> Result<BlockSuccess, ()> {
        if let SysCallSuccess::BlockStore(bsr) = succ {
            Ok(bsr)
        } else {
            Err(())
        }
    }

    /// Obtain information about the Block Storage Device.
    pub fn store_info() -> Result<StoreInfo, ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::StoreInfo);
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::StoreInfo(si) = resp {
            Ok(si)
        } else {
            Err(())
        }
    }

    /// Obtain information about a given block index on the Block Storage Device.
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

    /// Open a block for reading or writing.
    pub fn block_open(block: u32) -> Result<(), ()> {
        let req = SysCallRequest::BlockStore(BlockRequest::BlockOpen { block_idx: block });
        let resp = success_filter(try_syscall(req)?)?;

        if let BlockSuccess::BlockOpened = resp {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Read the contents of a given block, starting at a given offset.
    ///
    /// The offset is the number of bytes from the start of the block.
    /// The offset must be 4-byte aligned. The `dest_buf` must be four
    /// byte aligned.
    ///
    /// On success, the portion of data read from the block is returned.
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

    /// Write the contents to a given block, starting at a given byte offset.
    ///
    /// The offset is the number of bytes from the start of the block.
    /// The offset must be 4-byte aligned. The `bytes` buf must be four
    /// byte aligned.
    ///
    /// If this is the first write to a given block after opening, the entire
    /// block will be erased. Partial writes/rewrites are not currently
    /// supported. Subsequent reads will reflect the erased status.
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

    /// Close the given block, and update its metadata.
    ///
    /// The name may be any UTF-8 string, but must be less than 128 bytes in size,
    /// e.g. `name.as_bytes().len() <= 128`.
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
