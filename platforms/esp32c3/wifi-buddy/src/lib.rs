#![no_std]

pub use mnemos_esp32c3_core::*;

/// Initialize the heap.
///
/// # Safety
///
/// Only call this once!
pub unsafe fn heap_init() {
    extern "C" {
        // Note: this symbol is provided by `esp32c3-hal`'s linker script.
        static mut _heap_start: u32;
    }

    let heap_start = {
        let ptr = &_heap_start as *const _ as *mut u8;
        core::ptr::NonNull::new(ptr).expect(
            "why would the heap start address, given to us by the linker script, ever be null?",
        )
    };

    heap::init(heap_start);
}
