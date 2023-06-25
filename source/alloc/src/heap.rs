use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicBool, Ordering},
};

use linked_list_allocator::Heap;
use maitake::sync::{Mutex, WaitQueue};

static OOM_WAITER: WaitQueue = WaitQueue::new();
static INHIBIT_ALLOC: AtomicBool = AtomicBool::new(false);

pub async fn alloc(layout: Layout) -> NonNull<u8> {
    loop {
        unsafe {
            match NonNull::new(alloc::alloc::alloc(layout.clone())) {
                Some(nn) => return nn,
                None => {
                    let _ = OOM_WAITER.wait().await;
                    continue;
                }
            }
        }
    }
}

#[inline(always)]
pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    alloc::alloc::dealloc(ptr, layout)
}

pub trait UlAlloc {
    const INIT: Self;
    unsafe fn init(&self, start: NonNull<u8>, len: usize);
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8;
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout);
}

pub struct Mlla {
    mlla: Mutex<Heap>,
}

impl UlAlloc for Mlla {
    const INIT: Self = Mlla {
        mlla: Mutex::new(Heap::empty()),
    };

    #[inline]
    unsafe fn init(&self, start: NonNull<u8>, len: usize) {
        let mut heap = self.mlla.try_lock().unwrap();
        assert!(heap.size() == 0, "Already initialized the heap");
        heap.init(start.as_ptr(), len);
    }

    #[inline]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let mut heap = self.mlla.try_lock().unwrap();
        heap.allocate_first_fit(layout)
            .ok()
            .map_or(core::ptr::null_mut(), |allocation| allocation.as_ptr())
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        match NonNull::new(ptr) {
            Some(nn) => {
                let mut heap = self.mlla.try_lock().unwrap();
                heap.deallocate(nn, layout);
            }
            None => {
                debug_assert!(false, "Deallocating a null?");
                return;
            }
        }
    }
}

#[cfg(feature = "use-std")]
impl UlAlloc for std::alloc::System {
    const INIT: Self = std::alloc::System;

    unsafe fn init(&self, _start: NonNull<u8>, _len: usize) {
        panic!("Don't initialize the system allocator.");
    }

    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        <std::alloc::System as GlobalAlloc>::alloc(self, layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        <std::alloc::System as GlobalAlloc>::dealloc(self, ptr, layout)
    }
}

pub struct AHeap2<U> {
    allocator: U,
}

impl<U: UlAlloc> AHeap2<U> {
    pub const fn new() -> Self {
        Self { allocator: U::INIT }
    }

    pub unsafe fn init(&self, start: NonNull<u8>, len: usize) {
        self.allocator.init(start, len)
    }
}

unsafe impl<U: UlAlloc> GlobalAlloc for AHeap2<U> {
    #[inline(always)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if INHIBIT_ALLOC.load(Ordering::SeqCst) {
            // TODO
            return null_mut();
        }
        let ptr = self.allocator.alloc(layout);
        if ptr.is_null() {
            INHIBIT_ALLOC.store(true, Ordering::SeqCst); // TODO
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.allocator.dealloc(ptr, layout);
        INHIBIT_ALLOC.store(false, Ordering::SeqCst); // TODO
        OOM_WAITER.wake_all();
    }
}
