use alloc::alloc::GlobalAlloc;
use core::{mem::MaybeUninit, ptr::NonNull};
use esp_alloc::EspHeap;
use kernel::mnemos_alloc::heap::{MnemosAlloc, UnderlyingAllocator};

#[global_allocator]
static AHEAP: MnemosAlloc<UnderlyingEspHeap> = MnemosAlloc::new();

pub const HEAP_SIZE: usize = 1024 * 32;

/// Initialize the heap.
pub fn init() {
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();
    unsafe {
        let ptr = NonNull::new(HEAP.as_mut_ptr())
            .expect("HEAP static should never be null!")
            .cast::<u8>();
        AHEAP
            .init(ptr, HEAP_SIZE)
            .expect("heap initialized more than once!")
    }
}

struct UnderlyingEspHeap(EspHeap);

impl UnderlyingAllocator for UnderlyingEspHeap {
    /// A constant initializer of the allocator.
    ///
    /// May or may not require a call to [UnderlyingAllocator::init()] before the allocator
    /// is actually ready for use.
    //
    // clippy note: <https://rust-lang.github.io/rust-clippy/master/index.html#/declare_interior_mutable_const>
    //
    // > A “non-constant” const item is a legacy way to supply an initialized value to
    // > downstream static items (e.g., the std::sync::ONCE_INIT constant). In this
    // > case the use of const is legit, and this lint should be suppressed.
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
