//! The mnemos-alloc Heap types

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicBool, Ordering},
};

use linked_list_allocator::Heap;
use maitake::sync::{Mutex, WaitQueue};

/// # Mnemos Allocator
///
/// This is a wrapper type over an implementor of [UnderlyingAllocator].
///
/// This "inherits" any of the behaviors and safety requirements of the
/// chosen [UnderlyingAllocator], and in addition has two major special
/// behaviors that are intended to help respond gracefully to ephemeral
/// Out of Memory conditions
///
/// * On **alloc**:
///     * We check whether allocation is inhibited. If it is - a nullptr
///       is returned, regardless of whether there is sufficient room to
///       allocate the requested amount.
///     * If we are NOT inhibited, but are now out of memory (the underlying
///       allocator returned a nullptr), we inhibit further allocations until
///       the next deallocation occurs
/// * On **dealloc**:
///     * The "inhibit allocations" flag is cleared
///     * If any tasks are waiting on the "OOM" queue, they are ALL awoken if
///       the inhibit flag was previously set
///
/// These two details are intended to allow the "async allocation aware" types
/// defined in [crate::containers] to yield if allocation is not currently possible.
///
/// By wrapping the [UnderlyingAllocator], we allow non-async-aware allocations
/// (like those through [alloc::alloc::alloc()] or [alloc::alloc::dealloc()]) to
/// trigger these behaviors. However, non-async-aware allocations are still subject
/// to normal OOM handling, which typically means panicking.
pub struct MnemosAlloc<U> {
    allocator: U,
}

impl<U: UnderlyingAllocator> MnemosAlloc<U> {
    pub const fn new() -> Self {
        Self { allocator: U::INIT }
    }

    pub unsafe fn init(&self, start: NonNull<u8>, len: usize) {
        self.allocator.init(start, len)
    }
}

unsafe impl<U: UnderlyingAllocator> GlobalAlloc for MnemosAlloc<U> {
    #[inline(always)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if INHIBIT_ALLOC.load(Ordering::Acquire) {
            return null_mut();
        }
        let ptr = self.allocator.alloc(layout);
        if ptr.is_null() {
            INHIBIT_ALLOC.store(true, Ordering::Release);
        }
        ptr
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.allocator.dealloc(ptr, layout);
        let was_inhib = INHIBIT_ALLOC.swap(false, Ordering::AcqRel);
        if was_inhib {
            OOM_WAITER.wake_all();
        }
    }
}

/// A [WaitQueue] for tasks that would like to allocate, but the allocator is
/// currently in temporary OOM mode
static OOM_WAITER: WaitQueue = WaitQueue::new();

/// Flag to inhibit allocs. This ensures that allocations are served in a FIFO
/// order, so if there are 50 bytes left, then we get a sequence of alloc requests
/// like [64, 10, 30], none will be served until there is room for 64. This prevents
/// large allocations from being starved, at the cost of delaying small allocations
/// that *could* potentially succeed
static INHIBIT_ALLOC: AtomicBool = AtomicBool::new(false);

/// Asynchronously allocate with the given [Layout].
///
/// Analogous to [alloc::alloc::alloc()], but will never return a null pointer,
/// and will instead yield until allocation succeeds (which could theoretically
/// be never).
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

/// Immediately deallocate the given ptr + [Layout]
///
/// Safety: This has the same safety invariants as [alloc::alloc::dealloc()].
#[inline(always)]
pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    alloc::alloc::dealloc(ptr, layout)
}

/// "Underlying Allocator"" Trait
///
/// This trait serves to abstract over a general purpose [GlobalAlloc] implementation,
/// and allows `mnemos-alloc` to do "the right thing" when it comes to the async wrapper
/// types when used with any allocator.
///
/// [UnderlyingAllocator::alloc()] and [UnderlyingAllocator::dealloc()] must be implemented.
/// [UnderlyingAllocator::init()] may or may not be necessary, depending on your allocator.
///
/// ## Features
///
/// When the "use-std" feature of this crate is active, an implementation of
/// [UnderlyingAllocator] is provided for `std::alloc::System`.
pub trait UnderlyingAllocator {
    /// A constant initializer of the allocator.
    ///
    /// May or may not require a call to [UnderlyingAllocator::init()] before the allocator
    /// is actually ready for use.
    const INIT: Self;

    /// Initialize the allocator, if it is necessary to populate with a region of memory.
    unsafe fn init(&self, start: NonNull<u8>, len: usize);

    /// Allocate a region of memory
    ///
    /// SAFETY: The same as [GlobalAlloc::alloc()].
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8;

    /// Deallocate a region of memory
    ///
    /// SAFETY: The same as [GlobalAlloc::dealloc()].
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout);
}

/// A wrapper of [linked_list_allocator::Heap] that uses [maitake::sync::Mutex].
///
/// This should ONLY be used in a single threaded context, which also includes
/// NOT using it in interrupts.
///
/// If an allocation is attempted while the mutex is locked (e.g. we are pre-empted
/// by a thread/interrupt), this allocator will panic.
///
/// This allocator MUST be initialized with a call to [SingleThreadedLinkedListAllocator::init()]
/// before any allocations will succeed
pub struct SingleThreadedLinkedListAllocator {
    mlla: Mutex<Heap>,
}

impl UnderlyingAllocator for SingleThreadedLinkedListAllocator {
    const INIT: Self = SingleThreadedLinkedListAllocator {
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
impl UnderlyingAllocator for std::alloc::System {
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
