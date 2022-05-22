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

use serde::{Serialize, Deserialize};

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

    /// The top level SysCallRequest type. This is the type expected by the
    /// kernel when triggering a syscall.
    #[derive(Serialize, Deserialize)]
    pub enum SysCallRequest {
    //     Serial(SerialRequest<'a>),
        Time(TimeRequest),
    //     BlockStore(BlockRequest<'a>),
    //     System(SystemRequest<'a>),
        Gpio(GpioRequest),
    //     PcmSink(PcmSinkRequest),
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

    // /// Requests associated with system control.
    // #[derive(Serialize, Deserialize)]
    // pub enum SystemRequest<'a> {
    //     SetBootBlock {
    //         block: u32
    //     },
    //     Reset,
    //     FreeFutureBox {
    //         fb_ptr: u32,
    //         payload_size: u32,
    //         payload_align: u32,
    //     },
    //     Panic,
    //     RandFill {
    //         dest: SysCallSliceMut<'a>
    //     },
    // }

    // /// Requests associated with Virtual Serial Port operations.
    // #[derive(Serialize, Deserialize)]
    // pub enum SerialRequest<'a> {
    //     SerialOpenPort {
    //         port: u16,
    //     },
    //     SerialReceive {
    //         port: u16,
    //         dest_buf: SysCallSliceMut<'a>
    //     },
    //     SerialSend {
    //         port: u16,
    //         src_buf: SysCallSlice<'a>,
    //     },
    // }

    /// Requests associated with time.
    #[derive(Serialize, Deserialize)]
    pub enum TimeRequest {
        SleepMicros {
            us: u32,
        }
    }

    // /// Requests associated with the Block Storage device.
    // #[derive(Serialize, Deserialize)]
    // pub enum BlockRequest<'a> {
    //     StoreInfo,
    //     BlockInfo {
    //         block_idx: u32,
    //         name_buf: SysCallSliceMut<'a>
    //     },
    //     BlockOpen {
    //         block_idx: u32,
    //     },
    //     BlockRead {
    //         block_idx: u32,
    //         offset: u32,
    //         dest_buf: SysCallSliceMut<'a>,
    //     },
    //     BlockWrite {
    //         block_idx: u32,
    //         offset: u32,
    //         src_buf: SysCallSlice<'a>,
    //     },
    //     BlockClose {
    //         block_idx: u32,
    //         name: SysCallSlice<'a>,
    //         len: u32,
    //         kind: BlockKind,
    //     }
    // }

    // #[derive(Serialize, Deserialize)]
    // pub enum PcmSinkRequest {
    //     Enable,
    //     Disable,
    //     AllocateSampleBuffer {
    //         count: u32,
    //     }
    // }
}

/// Types used in syscall responses - from kernel to userspace
pub mod success {
    use super::*;

    /// The top level SysCallRequest type. This is the type expected by the
    /// userspace when obtaining the result of a successful system call.
    #[derive(Serialize, Deserialize)]
    pub enum SysCallSuccess {
        // Serial(SerialSuccess<'a>),
        Time(TimeSuccess),
        // BlockStore(BlockSuccess<'a>),
        // System(SystemSuccess<'a>),
        Gpio(GpioSuccess),
        // PcmSink(PcmSinkSuccess),
    }

    #[derive(Serialize, Deserialize)]
    pub enum GpioSuccess {
        ModeSet,
        ReadInput {
            is_high: bool,
        },
        OutputWritten,
    }

    // /// Success type for System level requests
    // #[derive(Serialize, Deserialize)]
    // pub enum SystemSuccess<'a> {
    //     BootBlockSet,
    //     Freed,
    //     RandFilled {
    //         dest: SysCallSliceMut<'a>
    //     }
    // }

    // /// Success type for Virtual Serial Port requests
    // #[derive(Serialize, Deserialize)]
    // pub enum SerialSuccess<'a> {
    //     PortOpened,
    //     DataReceived {
    //         dest_buf: SysCallSliceMut<'a>,
    //     },
    //     DataSent {
    //         remainder: Option<SysCallSlice<'a>>,
    //     },
    // }

    /// Success type for time related requests
    #[derive(Serialize, Deserialize)]
    pub enum TimeSuccess {
        SleptMicros {
            us: u32,
        },
    }

    // /// Information about a single Block Storage Device block
    // #[derive(Serialize, Deserialize)]
    // pub struct BlockInfo<'a>{
    //     pub length: u32,
    //     pub capacity: u32,
    //     pub kind: BlockKind,
    //     pub status: BlockStatus,
    //     pub name: Option<SysCallSlice<'a>>,
    // }

    // /// The current status of a given Block Storage Device block
    // #[derive(Serialize, Deserialize, Copy, Clone, PartialEq, Eq, Debug)]
    // pub enum BlockStatus {
    //     Idle,
    //     OpenNoWrites,
    //     OpenWritten,
    // }

    // /// Information about a Block Storage Device
    // #[derive(Serialize, Deserialize)]
    // pub struct StoreInfo {
    //     pub blocks: u32,
    //     pub capacity: u32,
    // }

    // /// Success type for Block Storage Device related requests
    // #[derive(Serialize, Deserialize)]
    // pub enum BlockSuccess<'a> {
    //     StoreInfo(StoreInfo),
    //     BlockInfo(BlockInfo<'a>),
    //     BlockOpened,
    //     BlockRead {
    //         dest_buf: SysCallSliceMut<'a>,
    //     },
    //     BlockWritten,
    //     BlockClosed,
    // }

    // #[derive(Serialize, Deserialize)]
    // pub enum PcmSinkSuccess {
    //     Enabled,
    //     Disabled,
    //     SampleBuffer {
    //         fut: SysCallFuture,
    //     }
    // }
}
