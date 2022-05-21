#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]

use core::sync::atomic::AtomicPtr;
use bbqueue_ipc::BBBuffer;

pub mod porcelain;
pub mod syscall;
pub mod bbqueue_ipc;

// This will always live at the TOP of the user memory region, and will be
// initialized by the kernel before
#[repr(C)]
pub struct SysCallRings {
    /// USER should take the PRODUCER
    /// KERNEL should take the CONSUMER
    pub user_to_kernel: AtomicPtr<BBBuffer>,

    /// USER should take the CONSUMER
    /// KERNEL should take the PRODUCER
    pub kernel_to_user: AtomicPtr<BBBuffer>,
}
