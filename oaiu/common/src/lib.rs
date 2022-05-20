#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]

use core::{sync::atomic::{AtomicPtr, AtomicUsize}, ptr::null_mut};

pub mod porcelain;
pub mod syscall;

// NOTE: These symbols are only public so the kernel doesn't have to
// redefine them. Don't touch. These will eventually go away.

#[link_section=".bridge.syscall_in.ptr"]
#[no_mangle]
#[used]
#[doc(hidden)]
pub static SYSCALL_IN_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());

#[link_section=".bridge.syscall_in.len"]
#[no_mangle]
#[used]
#[doc(hidden)]
pub static SYSCALL_IN_LEN: AtomicUsize = AtomicUsize::new(0);

#[link_section=".bridge.syscall_out.ptr"]
#[no_mangle]
#[used]
#[doc(hidden)]
pub static SYSCALL_OUT_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());

#[link_section=".bridge.syscall_out.len"]
#[no_mangle]
#[used]
#[doc(hidden)]
pub static SYSCALL_OUT_LEN: AtomicUsize = AtomicUsize::new(0);

