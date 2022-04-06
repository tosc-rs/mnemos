#![no_std]
#![no_main]

use userspace as _; // Panic handler
use core::sync::atomic::Ordering;

#[no_mangle]
fn entry() -> ! {
    // TODO: Prevent these from being optimized out?
    let _a = common::SYSCALL_IN_PTR.load(Ordering::SeqCst);
    let _b = common::SYSCALL_IN_LEN.load(Ordering::SeqCst);
    let _c = common::SYSCALL_OUT_PTR.load(Ordering::SeqCst);
    let _d = common::SYSCALL_OUT_LEN.load(Ordering::SeqCst);
    panic!();
}
