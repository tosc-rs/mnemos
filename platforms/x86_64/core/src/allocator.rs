use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};
use hal_core::{mem, BootInfo, VAddr};
use kernel::mnemos_alloc::heap::{MnemosAlloc, UnderlyingAllocator};
use mycelium_alloc::{buddy, bump};

/// 1k is enough for anyone.
pub const BUMP_SIZE: usize = 1024;

/// 32 free lists is enough for anyone.
const FREE_LISTS: usize = 32;

const MIN_HEAP_SIZE: usize = 32;

#[derive(Debug)]
pub struct Heap(());

#[global_allocator]
pub static AHEAP: MnemosAlloc<Heap> = MnemosAlloc::new();

pub(crate) static HEAP: buddy::Alloc<FREE_LISTS> = buddy::Alloc::new(MIN_HEAP_SIZE);
static BUMP: bump::Alloc<BUMP_SIZE> = bump::Alloc::new();

pub(crate) fn init(bootinfo: &impl BootInfo, vm_offset: VAddr) {
    HEAP.set_vm_offset(vm_offset);

    let mut regions = 0;
    let mut free_regions = 0;
    let mut free_bytes = 0;

    for region in bootinfo.memory_map() {
        let kind = region.kind();
        let size = region.size();
        tracing::info!(
            "  {:>10?} {:>15?} {:>15?} B",
            region.base_addr(),
            kind,
            size,
        );
        regions += 1;
        if region.kind() == mem::RegionKind::FREE {
            free_regions += 1;
            free_bytes += size;
            if unsafe { HEAP.add_region(region) }.is_err() {
                tracing::warn!("bad region");
            }
        }
    }

    assert!(
        free_regions > 0,
        "no free memory regions found (out of {regions} total)"
    );

    tracing::info!(
        "found {} memory regions, {} free regions ({} bytes)",
        regions,
        free_regions,
        free_bytes,
    );
}

impl UnderlyingAllocator for Heap {
    const INIT: Self = Self(());
    unsafe fn init(&self, _: NonNull<u8>, _: usize) {
        unimplemented!()
    }

    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // first, try to allocate from the real heap.
        let ptr = HEAP.alloc(layout);

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

        HEAP.dealloc(ptr, layout);
    }
}
