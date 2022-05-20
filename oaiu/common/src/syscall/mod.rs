//! System Call Types and low level methods
//!
//! These system call types act as the "wire format" between the kernel
//! and userspace.
//!
//! These types and functions are NOT generally used outside of creating
//! [`porcelain`](crate::porcelain) functions, or within the kernel itself.
//!
//! Consider using the [`porcelain`](crate::porcelain) functions directly
//! instead when making userspace system calls.
//!
//! ## WARNING!
//!
//! Care must be taken when modifying these types! Non-additive changes,
//! including ANY field reordering **MUST** be considered a breaking change!
//!
//! I have chosen NOT to mark these enums as `#[non_exhaustive]` as
//! Serde will already fail deserialization on an unknown enum variant.
//!
//! Breakages of downstream code causing non-exhaustive enum matching errors
//! due to added enum variants are NOT considered a "breaking change" at the
//! moment. If this is important to you, pin the exact `common` crate version
//! you plan to support, or open an issue to discuss changing this policy.

#[cfg(all(target_arch = "arm", target_os = "none"))]
use core::{sync::atomic::Ordering, ptr::null_mut, arch::asm};

use core::marker::PhantomData;
use serde::{Serialize, Deserialize};
use crate::syscall::{request::SysCallRequest, success::SysCallSuccess};

/// The kind of a given block
///
/// This lives outside of the Request/Success blocks as it used by both.
#[derive(Serialize, Deserialize, Eq, PartialEq, Copy, Clone, Debug)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum BlockKind {
    Unused,
    Storage,
    Program,
}

/// Types used in syscall requests - from userspace to kernel
pub mod request {
    use super::*;
    use crate::syscall::slice::{SysCallSlice, SysCallSliceMut};

    /// The top level SysCallRequest type. This is the type expected by the
    /// kernel when triggering a syscall.
    #[derive(Serialize, Deserialize)]
    pub enum SysCallRequest<'a> {
        Serial(SerialRequest<'a>),
        Time(TimeRequest),
        BlockStore(BlockRequest<'a>),
        System(SystemRequest<'a>),
        Gpio(GpioRequest),
        PcmSink(PcmSinkRequest),
    }

    #[derive(Serialize, Deserialize)]
    pub enum GpioMode {
        Disabled,
        InputFloating,
        InputPullUp,
        InputPullDown,
        OutputPushPull {
            is_high: bool
        },
    }

    #[derive(Serialize, Deserialize)]
    pub enum GpioRequest {
        SetMode {
            pin: u8,
            mode: GpioMode,
        },
        ReadInput {
            pin: u8,
        },
        WriteOutput {
            pin: u8,
            is_high: bool,
        }
    }

    impl GpioRequest {
        pub fn pin(&self) -> u8 {
            match self {
                GpioRequest::SetMode { pin, .. } => *pin,
                GpioRequest::ReadInput { pin } => *pin,
                GpioRequest::WriteOutput { pin, .. } => *pin,
            }
        }
    }

    /// Requests associated with system control.
    #[derive(Serialize, Deserialize)]
    pub enum SystemRequest<'a> {
        SetBootBlock {
            block: u32
        },
        Reset,
        FreeFutureBox {
            fb_ptr: u32,
            payload_size: u32,
            payload_align: u32,
        },
        Panic,
        RandFill {
            dest: SysCallSliceMut<'a>
        },
    }

    /// Requests associated with Virtual Serial Port operations.
    #[derive(Serialize, Deserialize)]
    pub enum SerialRequest<'a> {
        SerialOpenPort {
            port: u16,
        },
        SerialReceive {
            port: u16,
            dest_buf: SysCallSliceMut<'a>
        },
        SerialSend {
            port: u16,
            src_buf: SysCallSlice<'a>,
        },
    }

    /// Requests associated with time.
    #[derive(Serialize, Deserialize)]
    pub enum TimeRequest {
        SleepMicros {
            us: u32,
        }
    }

    /// Requests associated with the Block Storage device.
    #[derive(Serialize, Deserialize)]
    pub enum BlockRequest<'a> {
        StoreInfo,
        BlockInfo {
            block_idx: u32,
            name_buf: SysCallSliceMut<'a>
        },
        BlockOpen {
            block_idx: u32,
        },
        BlockRead {
            block_idx: u32,
            offset: u32,
            dest_buf: SysCallSliceMut<'a>,
        },
        BlockWrite {
            block_idx: u32,
            offset: u32,
            src_buf: SysCallSlice<'a>,
        },
        BlockClose {
            block_idx: u32,
            name: SysCallSlice<'a>,
            len: u32,
            kind: BlockKind,
        }
    }

    #[derive(Serialize, Deserialize)]
    pub enum PcmSinkRequest {
        Enable,
        Disable,
        AllocateSampleBuffer {
            count: u32,
        }
    }
}

/// Types used in syscall responses - from kernel to userspace
pub mod success {
    use super::*;
    use super::future::SysCallFuture;
    use crate::syscall::slice::{SysCallSlice, SysCallSliceMut};

    /// The top level SysCallRequest type. This is the type expected by the
    /// userspace when obtaining the result of a successful system call.
    #[derive(Serialize, Deserialize)]
    pub enum SysCallSuccess<'a> {
        Serial(SerialSuccess<'a>),
        Time(TimeSuccess),
        BlockStore(BlockSuccess<'a>),
        System(SystemSuccess<'a>),
        Gpio(GpioSuccess),
        PcmSink(PcmSinkSuccess),
    }

    #[derive(Serialize, Deserialize)]
    pub enum GpioSuccess {
        ModeSet,
        ReadInput {
            is_high: bool,
        },
        OutputWritten,
    }

    /// Success type for System level requests
    #[derive(Serialize, Deserialize)]
    pub enum SystemSuccess<'a> {
        BootBlockSet,
        Freed,
        RandFilled {
            dest: SysCallSliceMut<'a>
        }
    }

    /// Success type for Virtual Serial Port requests
    #[derive(Serialize, Deserialize)]
    pub enum SerialSuccess<'a> {
        PortOpened,
        DataReceived {
            dest_buf: SysCallSliceMut<'a>,
        },
        DataSent {
            remainder: Option<SysCallSlice<'a>>,
        },
    }

    /// Success type for time related requests
    #[derive(Serialize, Deserialize)]
    pub enum TimeSuccess {
        SleptMicros {
            us: u32,
        },
    }

    /// Information about a single Block Storage Device block
    #[derive(Serialize, Deserialize)]
    pub struct BlockInfo<'a>{
        pub length: u32,
        pub capacity: u32,
        pub kind: BlockKind,
        pub status: BlockStatus,
        pub name: Option<SysCallSlice<'a>>,
    }

    /// The current status of a given Block Storage Device block
    #[derive(Serialize, Deserialize, Copy, Clone, PartialEq, Eq, Debug)]
    pub enum BlockStatus {
        Idle,
        OpenNoWrites,
        OpenWritten,
    }

    /// Information about a Block Storage Device
    #[derive(Serialize, Deserialize)]
    pub struct StoreInfo {
        pub blocks: u32,
        pub capacity: u32,
    }

    /// Success type for Block Storage Device related requests
    #[derive(Serialize, Deserialize)]
    pub enum BlockSuccess<'a> {
        StoreInfo(StoreInfo),
        BlockInfo(BlockInfo<'a>),
        BlockOpened,
        BlockRead {
            dest_buf: SysCallSliceMut<'a>,
        },
        BlockWritten,
        BlockClosed,
    }

    #[derive(Serialize, Deserialize)]
    pub enum PcmSinkSuccess {
        Enabled,
        Disabled,
        SampleBuffer {
            fut: SysCallFuture,
        }
    }
}

pub mod future {
    use super::*;
    use core::sync::atomic::{AtomicU8, AtomicBool, AtomicPtr};

    // This gets leaked
    #[repr(C)]
    #[doc(hidden)]
    #[derive(Debug)]
    pub struct FutureBox<T> {
        // TODO: Should these fields be one atomic u32?

        // Current status. Should only be updated by the holder of
        // the exclusive token
        pub status: AtomicU8,

        // Reference count, including exclusive and shared handles
        pub refcnt: AtomicU8,

        // Is the exclusive handle taken?
        pub ex_taken: AtomicBool,

        // TODO: This is a pointer to the T in a HeapBox<T>.
        pub payload: AtomicPtr<T>,
    }

    pub mod status {
        /// Kernel is working, and should be allowed exclusive access,
        /// if it doesn't have it already.
        pub const KERNEL_ACCESS: u8 = 0;

        /// Userspace is working, and should be allowed exclusive access,
        /// if it doesn't have it already.
        pub const USERSPACE_ACCESS: u8 = 1;

        /// The future has completed (on either side), but the payload
        /// is no longer accessible.
        pub const COMPLETED: u8 = 2;

        /// This future encountered an error, and will never reach the
        /// completed stage. The payload is no longer accessible.
        pub const ERROR: u8 = 3;

        /// Used to signify a handle that will only ever pend error or completed
        pub const INVALID: u8 = 4;
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub enum SCFutureKind {
        Bytes {
            ptr: u32,
            len: u32,
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct SysCallFuture {
        // Pointer to the future box
        pub ptr_fb: u32,
        pub kind: SCFutureKind,
        pub is_exclusive: bool,
        pub payload_size: u32,
        pub payload_align: u32,
    }
}

/// Special types to encode slices across the system call boundary
///
/// These types are **not** typically expected to be used directly,
/// instead consider using the [porcelain functions][crate::porcelain]
/// available instead, which avoid the need for low level interaction.
///
/// ## Safety Note
///
/// Using Serde on fields with unsafe side effects is
/// likely a Bad Idea^TM. I'm guessing you could create arbitrary
/// slice references safely, triggering UB.
///
/// At the moment - **don't do that**. Or if you do, don't expect stability
/// or safety.
///
/// The "correct" answer is likely to have public and private types,
/// where the userspace public types DON'T implement serde and private
/// ones that do.
///
/// For now: YOLO. User beware if you try to do something 'clever'.
pub mod slice {
    use super::*;

    /// An Immutable System Call Slice
    ///
    /// This represents an immutable slice of bytes, e.g. `&[u8]`, across
    /// a system call boundary.
    ///
    /// This type is typically created by calling `.into()` on a slice.
    ///
    /// The lifetime of the original slice is maintained by the lifetime
    /// parameter `'a`.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use common::syscall::slice::SysCallSlice;
    /// let sli: &[u8] = &[0, 1, 2, 3];
    /// let scs: SysCallSlice<'_> = sli.into();
    /// ```
    #[derive(Serialize, Deserialize)]
    pub struct SysCallSlice<'a> {
        pub(crate) ptr: u32,
        pub(crate) len: u32,
        _pdlt: PhantomData<&'a [u8]>,
    }

    impl<'a> SysCallSlice<'a> {
        /// Consumes the `SysCallSlice`, returning it to a `&[u8]`.
        ///
        /// ## SAFETY
        ///
        /// This function should only be called on a `SysCallSlice` that was obtained
        /// either from the kernel, or that was converted from a slice.
        pub unsafe fn to_slice(self) -> &'a [u8] {
            core::slice::from_raw_parts(self.ptr as *const u8, self.len as usize)
        }
    }

    impl<'a> From<&'a [u8]> for SysCallSlice<'a> {
        fn from(sli: &'a [u8]) -> Self {
            Self {
                ptr: sli.as_ptr() as u32,
                len: sli.len() as u32,
                _pdlt: PhantomData,
            }
        }
    }

    /// A Mutable System Call Slice
    ///
    /// This represents a mutable slice of bytes, e.g. `&mut [u8]`, across
    /// a system call boundary.
    ///
    /// This type is typically created by calling `.into()` on a mutable slice.
    ///
    /// The lifetime of the original slice is maintained by the lifetime
    /// parameter `'a`.
    ///
    /// Additionally, a `SysCallSliceMut` can be converted into a `SysCallSlice`,
    /// though the reverse is not possible.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use common::syscall::slice::{SysCallSliceMut, SysCallSlice};
    /// let sli: &mut [u8] = &mut [0, 1, 2, 3];
    /// let scs: SysCallSliceMut<'_> = sli.into();
    /// let scs: SysCallSlice<'_> = scs.into();
    /// ```
    #[derive(Serialize, Deserialize)]
    pub struct SysCallSliceMut<'a> {
        pub(crate) ptr: u32,
        pub(crate) len: u32,
        _pdlt: PhantomData<&'a mut [u8]>,
    }

    impl<'a> SysCallSliceMut<'a> {
        /// Consumes the `SysCallSliceMut`, returning it to a `&mut [u8]`.
        ///
        /// ## SAFETY
        ///
        /// This function should only be called on a `SysCallSliceMut` that was obtained
        /// either from the kernel, or that was converted from a slice.
        pub unsafe fn to_slice_mut(self) -> &'a mut [u8] {
            core::slice::from_raw_parts_mut(self.ptr as *const u8 as *mut u8, self.len as usize)
        }
    }

    impl<'a> From<&'a mut [u8]> for SysCallSliceMut<'a> {
        fn from(sli: &'a mut [u8]) -> Self {
            Self {
                ptr: sli.as_ptr() as u32,
                len: sli.len() as u32,
                _pdlt: PhantomData,
            }
        }
    }

    impl<'a> From<SysCallSliceMut<'a>> for SysCallSlice<'a> {
        fn from(sli: SysCallSliceMut<'a>) -> Self {
            Self {
                ptr: sli.ptr,
                len: sli.len,
                _pdlt: PhantomData,
            }
        }
    }

}

/// Perform a failable system call
///
/// Take a system call request, and return a result containing either
/// a successful response, or an indeterminite error.
///
/// At the moment, this is limited to requests and responses with a maximum
/// serialized size of 128 bytes. This function handles the serialization
/// and deserialization of the request and response automatically.
///
/// (At least) 256 bytes of stack will be used to hold the serialized request
/// and response parameters
///
/// This function is typically only used for creating relevant
/// [porcelain functions][crate::porcelain]. Consider using those instead.
pub fn try_syscall<'a>(req: SysCallRequest<'a>) -> Result<SysCallSuccess<'a>, ()> {
    let mut inp_buf = [0u8; 64];
    let mut out_buf = [0u8; 64];
    let iused = postcard::to_slice(&req, &mut inp_buf).map_err(drop)?;
    let oused = raw_syscall(iused, &mut out_buf)?;
    let result = postcard::from_bytes(oused).map_err(drop)?;
    Ok(result)
}

/// Perform a "raw" syscall, with a given input and output buffer.
///
/// The `input` buffer must contain a valid serialized request prior to calling
/// this function. The `output` buffer must contain sufficient space for the
/// serialized response.
///
/// This function places the slice into the correct location (currently an
/// AtomicPtr/AtomicUsize pair), and triggers a Cortex M `SVCall 0` instruction,
/// which prompts the kernel handler to run.
#[cfg(all(target_arch = "arm", target_os = "none"))]
fn raw_syscall<'i, 'o>(input: &'i [u8], output: &'o mut [u8]) -> Result<&'o mut [u8], ()> {
    let in_ptr = input.as_ptr() as *mut u8;

    // Try to atomically swap the in ptr for our input parameter. If this fails,
    // it means another syscall is in progress, and we should try again later.
    //
    // An "idle" syscall state is represented as a null pointer in the input
    // field.
    //
    // TODO: Should we just spin on this? Probably doesn't matter until we have
    // pre-emption, if ever...
    crate::SYSCALL_IN_PTR
        .compare_exchange(
            null_mut(),
            in_ptr,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .map_err(drop)?;

    // We've made it past the hurdle! Fill the rest of the buffers, then trigger
    // the svc call
    crate::SYSCALL_IN_LEN.store(input.len(), Ordering::SeqCst);
    crate::SYSCALL_OUT_PTR.store(output.as_ptr() as *mut u8, Ordering::SeqCst);
    crate::SYSCALL_OUT_LEN.store(output.len(), Ordering::SeqCst);

    unsafe {
        asm!("svc 0");
    }

    // Now we need to grab the output length, then reset all fields.
    let new_out_len = crate::SYSCALL_OUT_LEN.swap(0, Ordering::SeqCst);
    crate::SYSCALL_OUT_PTR.store(null_mut(), Ordering::SeqCst);
    crate::SYSCALL_IN_LEN.store(0, Ordering::SeqCst);
    crate::SYSCALL_IN_PTR.store(null_mut(), Ordering::SeqCst);

    if new_out_len == 0 {
        // This is bad. Just report it as an error for now
        Err(())
    } else {
        Ok(&mut output[..new_out_len])
    }
}

// Shim for testing/host builds
#[cfg(not(all(target_arch = "arm", target_os = "none")))]
fn raw_syscall<'i, 'o>(_input: &'i [u8], _output: &'o mut [u8]) -> Result<&'o mut [u8], ()> {
    unimplemented!("Testing shim!")
}
