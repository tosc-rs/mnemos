#![no_std]
#![no_main]

#[link_section = ".anachro_table.entry_point"]
#[no_mangle]
pub static __ENTRY_POINT: fn() -> ! = hello;

// This does not help with the "used" things
use userspace as _;

// Having these here makes the code compile, even as "duplicate symbols!"
//
// #[link_section=".bridge.syscall_in.ptr"]
// #[no_mangle]
// pub static SYSCALL_IN_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());
//
// #[link_section=".bridge.syscall_in.len"]
// #[no_mangle]
// pub static SYSCALL_IN_LEN: AtomicUsize = AtomicUsize::new(0);
//
// #[link_section=".bridge.syscall_out.ptr"]
// #[no_mangle]
// pub static SYSCALL_OUT_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());
//
// #[link_section=".bridge.syscall_out.len"]
// #[no_mangle]
// pub static SYSCALL_OUT_LEN: AtomicUsize = AtomicUsize::new(0);

static CONTENT: AtomicU32 = AtomicU32::new(0xACACACAC);

fn hello() -> ! {
    // let a = userspace::SYSCALL_IN_PTR.load(Ordering::SeqCst);
    // let b = userspace::SYSCALL_IN_LEN.load(Ordering::SeqCst);
    // let c = userspace::SYSCALL_OUT_PTR.load(Ordering::SeqCst);
    // let d = userspace::SYSCALL_OUT_LEN.load(Ordering::SeqCst);
    // let x = CONTENT.load(Ordering::SeqCst);
    // panic!("{:?} {} {:?} {} {}", a, b, c, d, x);
    panic!();
}

use core::panic::PanicInfo;
use core::ptr::null_mut;
use core::sync::atomic::{self, Ordering, AtomicU32, AtomicPtr, AtomicUsize};

#[inline(never)]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        atomic::compiler_fence(Ordering::SeqCst);
    }
}

/*

MEMORY
{
  APP   : ORIGIN = 0x20000000, LENGTH = 64K
  ARAM  : ORIGIN = 0x20010000, LENGTH = 64K
}

.anachro_table ORIGIN(APP) :
{
  /* Headers for the header gods! */
  LONG(_stack_start);
  LONG(__srodata);
  LONG(__erodata);
  LONG(__sdata);
  LONG(__edata);
  LONG(__sbss);
  LONG(__ebss);

  /* Reset vector */
  KEEP(*(.anachro_table.entry_point)); /* this is the `__ENTRY_POINT` symbol */
  __reset_vector = .;

} > APP

Disassembly of section .anachro_table:

20000000 <userspace::hello-0x18>:
20000000:       20020000        ;
20000004:       200000e0        ;
20000008:       20010000        ;
2000000c:       20010000        ;
20000010:       20010000        ;
20000014:       20010000        ;

arm-none-eabi-size target/thumbv7em-none-eabihf/release/userspace
   text    data     bss     dec     hex filename
    228       0       0     228      e4 target/thumbv7em-none-eabihf/release/userspace



*/
