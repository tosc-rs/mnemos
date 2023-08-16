#![no_std]

extern crate alloc;

use core::{panic::PanicInfo, ptr::NonNull};
use kernel::mnemos_alloc::heap::{MnemosAlloc, SingleThreadedLinkedListAllocator};
use mnemos_d1_core::{trap::Trap, Ram, D1};

#[global_allocator]
static AHEAP: MnemosAlloc<SingleThreadedLinkedListAllocator> = MnemosAlloc::new();

/// Initialize the heap.
///
/// # Safety
///
/// Only call this once!
pub unsafe fn initialize_heap<const HEAP_SIZE: usize>(buf: &'static Ram<HEAP_SIZE>) {
    AHEAP.init(NonNull::new(buf.as_ptr()).unwrap(), HEAP_SIZE);
}

#[panic_handler]
fn handler(info: &PanicInfo) -> ! {
    D1::handle_panic(info)
}

#[export_name = "ExceptionHandler"]
fn exception_handler(trap_frame: &riscv_rt::TrapFrame) -> ! {
    match Trap::from_mcause().expect("mcause should never be invalid") {
        Trap::Interrupt(int) => {
            unreachable!("the exception handler should only recieve exception traps, but got {int}")
        }
        Trap::Exception(exn) => {
            let mepc = riscv::register::mepc::read();
            panic!("CPU exception: {exn} ({exn:#X}) at {mepc:#X}\n\n{trap_frame:?}")
        }
    }
}
