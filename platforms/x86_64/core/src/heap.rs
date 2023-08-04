use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};
use kernel::mnemos_alloc::heap::{MnemosAlloc, UnderlyingAllocator};
use mycelium_alloc::{buddy, bump};

#[derive(Debug)]
pub struct Heap(());

/// 64k is enough for anyone.
pub const BUMP_SIZE: usize = 1024;

/// 32 free lists is enough for anyone.
const FREE_LISTS: usize = 32;

const MIN_HEAP_SIZE: usize = 32;

#[global_allocator]
pub static AHEAP: MnemosAlloc<Heap> = MnemosAlloc::new();

pub(crate) static ALLOC: buddy::Alloc<FREE_LISTS> = buddy::Alloc::new(MIN_HEAP_SIZE);

static BUMP: bump::Alloc<BUMP_SIZE> = bump::Alloc::new();

impl UnderlyingAllocator for Heap {
    const INIT: Self = Self(());
    unsafe fn init(&self, start: NonNull<u8>, len: usize) {
        unimplemented!()
    }

    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // first, try to allocate from the real heap.
        let ptr = ALLOC.alloc(layout);

        if ptr.is_null() {
            // heap is uninitialized, fall back to the bump region.
            return BUMP.alloc(layout);
        }

        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // if this is in the bump region, just leak it.
        if BUMP.owns(ptr) {
            return;
        }

        ALLOC.dealloc(ptr, layout);
    }
}
