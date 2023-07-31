use alloc::alloc::GlobalAlloc;
use core::ptr::NonNull;
use esp_alloc::EspHeap;
use kernel::mnemos_alloc::heap::{MnemosAlloc, UnderlyingAllocator};

#[global_allocator]
static AHEAP: MnemosAlloc<UnderlyingEspHeap> = MnemosAlloc::new();

pub const HEAP_SIZE: usize = 1024 * 32;

/// Initialize the heap.
///
/// # Safety
///
/// Only call this once!
pub unsafe fn init(heap_start: NonNull<u8>) {
    unsafe {
        AHEAP.init(heap_start, HEAP_SIZE);
    }
}

struct UnderlyingEspHeap(EspHeap);

impl UnderlyingAllocator for UnderlyingEspHeap {
    /// A constant initializer of the allocator.
    ///
    /// May or may not require a call to [UnderlyingAllocator::init()] before the allocator
    /// is actually ready for use.
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = UnderlyingEspHeap(EspHeap::empty());

    /// Initialize the allocator, if it is necessary to populate with a region
    /// of memory.
    ///
    /// # Safety
    ///
    /// This function requires the caller to uphold the following invariants:
    ///
    /// - The memory region starting at `start` and ending at `start + len` may
    ///   not be accessed except through pointers returned by this allocator.
    /// - The end of the memory region (`start + len`) may not exceed the
    ///   physical memory available on the device.
    /// - The memory region must not contain memory regions used for
    ///   memory-mapped IO.
    unsafe fn init(&self, start: NonNull<u8>, len: usize) {
        self.0.init(start.as_ptr(), len)
    }

    /// Allocate a region of memory
    ///
    /// # Safety
    ///
    /// The same as [GlobalAlloc::alloc()].
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        self.0.alloc(layout)
    }

    /// Deallocate a region of memory
    ///
    /// # Safety
    ///
    /// The same as [GlobalAlloc::dealloc()].
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.0.dealloc(ptr, layout)
    }
}
