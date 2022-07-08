#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![allow(clippy::missing_safety_doc)]

use bbqueue_ipc::BBBuffer;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicUsize};

// TODO: Put this into a linker section
pub static K2U_RING: AtomicPtr<BBBuffer> = AtomicPtr::new(null_mut());
pub static U2K_RING: AtomicPtr<BBBuffer> = AtomicPtr::new(null_mut());
pub static HEAP_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());
pub static HEAP_LEN: AtomicUsize = AtomicUsize::new(0);

// TODO: Move me to mstd
// pub mod porcelain;
pub mod bbqueue_ipc;
pub mod boxes;
pub mod syscall;

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
