#![cfg_attr(not(feature = "use-std"), no_std)]

pub mod containers;
pub mod heap;
pub mod node;

extern crate alloc;


pub mod fornow {
    use core::{alloc::GlobalAlloc, ptr::{NonNull, null_mut}, sync::atomic::{AtomicBool, Ordering}};

    use linked_list_allocator::Heap;
    use maitake::sync::{Mutex, WaitQueue};

    static OOM_WAITER: WaitQueue = WaitQueue::new();
    static INHIBIT_ALLOC: AtomicBool = AtomicBool::new(false);

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

    }

    impl<U: UlAlloc> AHeap2<U> {
        pub const fn new() -> Self {
            Self {
                allocator: U::INIT,
            }
        }

        pub unsafe fn init(&self, start: NonNull<u8>, len: usize) {
            self.allocator.init(start, len)
        }
    }

    unsafe impl<U: UlAlloc> GlobalAlloc for AHeap2<U> {
        #[inline(always)]
        unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
            if INHIBIT_ALLOC.load(Ordering::SeqCst) { // TODO
                return null_mut();
            }
            let ptr = self.allocator.alloc(layout);
            if ptr.is_null() {
                INHIBIT_ALLOC.store(false, Ordering::SeqCst); // TODO
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
            self.allocator.dealloc(ptr, layout);
            INHIBIT_ALLOC.store(false, Ordering::SeqCst); // TODO
            OOM_WAITER.wake_all();
        }
    }

    pub mod collections {
        use core::{ops::Deref, alloc::Layout};

        use super::OOM_WAITER;

        // Arc

        pub struct Arc<T> {
            inner: alloc::sync::Arc<T>,
        }

        impl<T> Arc<T> {
            pub async fn new(t: T) -> Self {
                // TODO: Async way of allocating Arc?
                Self { inner: alloc::sync::Arc::new(t) }
            }
        }

        impl<T> Deref for Arc<T> {
            type Target = alloc::sync::Arc<T>;

            #[inline(always)]
            fn deref(&self) -> &Self::Target {
                &self.inner
            }
        }

        // FixedVec

        pub struct FixedVec<T> {
            inner: alloc::vec::Vec<T>,
        }

        impl<T> FixedVec<T> {
            pub async fn new(capacity: usize) -> Self {
                let layout = Layout::array::<T>(capacity).unwrap();
                loop {
                    let ptr = unsafe {
                        alloc::alloc::alloc(layout.clone())
                    };
                    if ptr.is_null() {
                        let _ = OOM_WAITER.wait().await;
                        continue;
                    }
                    unsafe {
                        return FixedVec {
                            inner: Vec::from_raw_parts(ptr.cast(), 0, capacity)
                        };
                    }
                }

            }

            #[inline]
            pub fn try_push(&mut self, t: T) -> Result<(), T> {
                if self.is_full() {
                    Err(t)
                } else {
                    self.inner.push(t);
                    Ok(())
                }
            }

            #[inline]
            pub fn try_extend_from_slice(&mut self, sli: &[T]) -> Result<(), ()>
            where
                T: Clone,
            {
                let new_len = match self.inner.len().checked_add(sli.len()) {
                    Some(c) => c,
                    None => return Err(()),
                };

                if new_len >= self.inner.capacity() {
                    return Err(());
                }

                self.inner.extend_from_slice(sli);
                Ok(())
            }

            #[inline]
            pub fn as_vec(&self) -> &alloc::vec::Vec<T> {
                &self.inner
            }

            /// Get inner mutable vec
            ///
            /// SAFETY:
            ///
            /// You must not do anything that could realloc or increase the capacity.
            /// We want an exact upper limit.
            ///
            /// This would not be memory unsafe, but would violate the invariants of [FixedVec],
            /// which is supposed to have a fixed upper size.
            #[inline]
            pub unsafe fn as_vec_mut(&mut self) -> &mut alloc::vec::Vec<T> {
                &mut self.inner
            }

            #[inline]
            pub fn as_slice(&self) -> &[T] {
                &self.inner
            }

            #[inline]
            pub fn as_slice_mut(&mut self) -> &mut [T] {
                &mut self.inner
            }

            #[inline]
            pub fn clear(&mut self) {
                self.inner.clear();
            }

            #[inline]
            pub fn is_full(&self) -> bool {
                self.inner.len() == self.inner.capacity()
            }
        }
    }
}

