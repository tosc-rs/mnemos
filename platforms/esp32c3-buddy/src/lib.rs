#![no_std]
extern crate alloc;

pub mod drivers;

use core::ptr::NonNull;
use kernel::mnemos_alloc::heap::{MnemosAlloc, SingleThreadedLinkedListAllocator};

#[global_allocator]
static AHEAP: MnemosAlloc<SingleThreadedLinkedListAllocator> = MnemosAlloc::new();

pub const HEAP_SIZE: usize = 32 * 1024;

/// Initialize the heap.
///
/// # Safety
///
/// Only call this once!
pub unsafe fn init_heap() {
    extern "C" {
        // this is defined by `esp32c3-hal`
        static mut _heap_start: u32;
    }

    unsafe {
        let heap_start = NonNull::from(&mut _heap_start).cast();
        AHEAP.init(heap_start, HEAP_SIZE);
    }
}
