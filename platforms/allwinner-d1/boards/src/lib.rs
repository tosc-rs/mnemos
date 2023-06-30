#![no_std]

extern crate alloc;

use mnemos_d1_core::{
    kernel::mnemos_alloc::{MnemosAlloc, SingleThreadedLinkedListAllocator},
    Ram, D1,
};

#[global_allocator]
static AHEAP: MnemosAlloc<SingleThreadedLinkedListAllocator> = MnemosAlloc::new();

/// Initialize the heap.
///
/// # Safety
///
/// Only call this once!
pub unsafe fn initialize_heap<const HEAP_SIZE: usize>(buf: &'static Ram<HEAP_SIZE>) {
    AHEAP.init(NonNull::new(AHEAP_BUF.as_ptr()).unwrap(), HEAP_SIZE);
}

#[panic_handler]
fn handler(info: &PanicInfo) -> ! {
    D1::handle_panic(info)
}
