#![no_std]

use core::{sync::atomic::{AtomicPtr, AtomicUsize}, ptr::null_mut};

#[link_section=".bridge.syscall_in.ptr"]
#[no_mangle]
#[used]
pub static SYSCALL_IN_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());

#[link_section=".bridge.syscall_in.len"]
#[no_mangle]
#[used]
pub static SYSCALL_IN_LEN: AtomicUsize = AtomicUsize::new(0);

#[link_section=".bridge.syscall_out.ptr"]
#[no_mangle]
#[used]
pub static SYSCALL_OUT_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());

#[link_section=".bridge.syscall_out.len"]
#[no_mangle]
#[used]
pub static SYSCALL_OUT_LEN: AtomicUsize = AtomicUsize::new(0);

