#![cfg_attr(not(feature = "use-std"), no_std)]

pub mod containers;
pub mod heap;
pub mod node;


pub mod fornow {
    use core::{alloc::GlobalAlloc, ptr::NonNull, sync::atomic::{AtomicBool, Ordering}};

    use linked_list_allocator::Heap;
    use maitake::sync::{Mutex, WaitQueue};

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
        const INIT: Self = Mlla { mlla: Mutex::new(Heap::empty()) };

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
                },
                None => {
                    debug_assert!(false, "Deallocating a null?");
                    return;
                },
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
        oom_waiter: WaitQueue,
        inhibit_alloc: AtomicBool,
    }

    impl<U: UlAlloc> AHeap2<U> {
        pub const fn new() -> Self {
            Self {
                allocator: U::INIT,
                oom_waiter: WaitQueue::new(),
                inhibit_alloc: AtomicBool::new(false),
            }
        }

        pub unsafe fn init(&self, start: NonNull<u8>, len: usize) {
            self.allocator.init(start, len)
        }
    }

    unsafe impl<U: UlAlloc> GlobalAlloc for AHeap2<U> {
        #[inline(always)]
        unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
            self.allocator.alloc(layout)
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
            self.allocator.dealloc(ptr, layout);
            self.inhibit_alloc.store(false, Ordering::Relaxed);
            self.oom_waiter.wake_all();
        }
    }
}

