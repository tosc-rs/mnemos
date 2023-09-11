//! The mnemos-alloc Heap types

use core::{
    alloc::{GlobalAlloc, Layout},
    hint,
    ptr::{null_mut, NonNull},
};

use linked_list_allocator::Heap;
use maitake::sync::{Mutex, WaitQueue};
#[cfg(feature = "stats")]
use portable_atomic::AtomicU16;
use portable_atomic::{AtomicBool, AtomicUsize, Ordering::*};

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

    /// The total size of the heap, in bytes.
    heap_size: AtomicUsize,

    /// Tracks heap statistics.
    #[cfg(feature = "stats")]
    stats: stats::Stats,
}

/// Errors returned by [`MnemosAlloc::init`].
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InitError {
    /// The heap has already been initialized.
    AlreadyInitialized,
}

#[cfg(feature = "stats")]
pub use self::stats::State;

impl<U: UnderlyingAllocator> MnemosAlloc<U> {
    const INITIALIZING: usize = usize::MAX;

    pub const fn new() -> Self {
        Self {
            allocator: U::INIT,
            heap_size: AtomicUsize::new(0),

            #[cfg(feature = "stats")]
            stats: stats::Stats::new(),
        }
    }

    /// Initialize the allocator, with a heap of size `len` starting at `start`.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`()`]`)` if the heap was successfully initialized.
    /// - [`Err`]`(`[`InitError::AlreadyInitialized`]`)` if this method has
    ///   already been called to initialize the heap.
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
    pub unsafe fn init(&self, start: NonNull<u8>, len: usize) -> Result<(), InitError> {
        match self
            .heap_size
            .compare_exchange(0, Self::INITIALIZING, AcqRel, Acquire)
        {
            // another CPU core is initializing the heap, so we must wait until
            // it has been initialized, to prevent this core from trying to use
            // the heap.
            Err(val) if val == Self::INITIALIZING => {
                while self.heap_size.load(Acquire) == Self::INITIALIZING {
                    hint::spin_loop();
                }
                return Err(InitError::AlreadyInitialized);
            }
            // the heap has already been initialized, so we return an error. it
            // can now safely be used by this thread.
            Err(_) => return Err(InitError::AlreadyInitialized),
            // we can now initialize the heap!
            Ok(_) => {}
        }

        // actually initialize the heap
        self.allocator.init(start, len);

        self.heap_size.compare_exchange(Self::INITIALIZING, len, AcqRel, Acquire)
            .expect("if we changed the heap state to INITIALIZING, no other CPU core should have changed its state");
        Ok(())
    }

    /// Returns the total size of the heap in bytes, including allocated space.
    ///
    /// The current free space remaining can be calculated by subtracting this
    /// value from [`self.allocated_size()`].
    #[must_use]
    pub fn total_size(&self) -> usize {
        self.heap_size.load(Acquire)
    }
}

unsafe impl<U: UnderlyingAllocator> GlobalAlloc for MnemosAlloc<U> {
    #[inline(always)]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if INHIBIT_ALLOC.load(Acquire) {
            return null_mut();
        }

        #[cfg(feature = "stats")]
        let _allocating = stats::start_context(&self.stats.allocating);

        let ptr = self.allocator.alloc(layout);
        if ptr.is_null() {
            INHIBIT_ALLOC.store(true, Release);
            #[cfg(feature = "stats")]
            {
                self.stats.alloc_oom_count.fetch_add(1, Release);
            }
        } else {
            #[cfg(feature = "stats")]
            {
                self.stats.allocated.fetch_add(layout.size(), Release);
                self.stats.alloc_success_count.fetch_add(1, Release);
            }
        }
        ptr
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        #[cfg(feature = "stats")]
        let _allocating = stats::start_context(&self.stats.deallocating);

        self.allocator.dealloc(ptr, layout);

        #[cfg(feature = "stats")]
        {
            self.stats.allocated.fetch_sub(layout.size(), Release);
            self.stats.dealloc_count.fetch_add(1, Release);
        }

        let was_inhib = INHIBIT_ALLOC.swap(false, AcqRel);
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
            match NonNull::new(alloc::alloc::alloc(layout)) {
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
/// # Safety
///
/// This has the same safety invariants as [alloc::alloc::dealloc()].
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
    unsafe fn init(&self, start: NonNull<u8>, len: usize);

    /// Allocate a region of memory
    ///
    /// # Safety
    ///
    /// The same as [GlobalAlloc::alloc()].
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8;

    /// Deallocate a region of memory
    ///
    /// # Safety
    ///
    /// The same as [GlobalAlloc::dealloc()].
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
    // This constant is used as an initializer, so the interior mutability is
    // not an issue.
    #[allow(clippy::declare_interior_mutable_const)]
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
                debug_assert!(false, "Deallocating a null?")
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

#[cfg(feature = "stats")]
mod stats {
    use super::*;

    #[derive(Debug)]
    #[cfg(feature = "stats")]
    pub(super) struct Stats {
        /// The total amount of memory currently allocated, in bytes.
        pub(super) allocated: AtomicUsize,

        /// A count of heap allocation attempts that have been completed
        /// successfully.
        pub(super) alloc_success_count: AtomicUsize,

        /// A count of heap allocation attempts that have failed because the heap
        /// was at capacity.
        pub(super) alloc_oom_count: AtomicUsize,

        /// A count of the number of times an allocation has been deallocated.
        pub(super) dealloc_count: AtomicUsize,

        /// A count of the total number of current allocation attempts.
        pub(super) allocating: AtomicU16,

        /// A count of the total number of current deallocation attempts.
        pub(super) deallocating: AtomicU16,
    }

    /// A snapshot of the current state of the heap.
    #[derive(Debug, Copy, Clone)]
    #[non_exhaustive]
    pub struct State {
        /// A count of the total number of concurrently executing calls to
        /// [`alloc`].
        ///
        /// If this is 0, no CPU cores are currently allocating.
        pub allocating: u16,

        /// A count of the total number of concurrently executing calls to
        /// [`dealloc`].
        ///
        /// If this is 0, no CPU cores are currently allocating.
        pub deallocating: u16,

        /// If this is `true`, an allocation request could not be satisfied
        /// because there was insufficient memory. That allocation request may
        /// be queued.
        pub is_oom: bool,

        /// The total size of the heap, in bytes. This includes memory
        /// that is currently allocated.
        pub total_bytes: usize,

        /// The amount of memory currently allocated, in bytes.
        pub allocated_bytes: usize,

        /// The total number of times an allocation attempt has
        /// succeeded, over the lifetime of this heap.
        pub alloc_success_count: usize,

        /// The total number of times an allocation attempt could not be
        /// fulfilled because there was insufficient space, over the lifetime of
        /// this heap.
        pub alloc_oom_count: usize,

        /// The total number of times an allocation has been freed, over the
        /// lifetime of this heap.
        pub dealloc_count: usize,
    }

    impl<U> MnemosAlloc<U> {
        /// Returns a snapshot of the current state of the heap.
        ///
        /// This returns a struct containing all available heap metrics at the
        /// current point in time. It permits calculating derived metrics, such
        /// as [`State::free_bytes`], [`State::alloc_attempt_count`], and
        /// [`State::live_alloc_count`], which are calculated using the values
        /// of other heap statistics.
        ///
        /// Taking a single snapshot ensures that no drift occurs between these
        /// metrics. For example, if we were to call
        /// [`Self::alloc_success_count()`], and then later attempt to calculate
        /// the number of live allocations by subtracting the value of
        /// [`Self::dealloc_count()`] from a subsequent call to
        /// [`Self::alloc_success_count()`], additional concurrent allocations
        /// may have occurred between the first time the success count was
        /// loaded and the second. Taking one snapshot of all metrics ensures
        /// that no drift occurs, because the snapshot contains all heap metrics
        /// at the current point in time.
        #[must_use]
        #[inline]
        pub fn state(&self) -> State {
            State {
                allocating: self.stats.allocating.load(Acquire),
                deallocating: self.stats.deallocating.load(Acquire),
                is_oom: INHIBIT_ALLOC.load(Acquire),
                total_bytes: self.total_bytes(),
                allocated_bytes: self.allocated_bytes(),
                alloc_success_count: self.alloc_success_count(),
                alloc_oom_count: self.alloc_oom_count(),
                dealloc_count: self.dealloc_count(),
            }
        }

        /// Returns the total amount of memory currently allocated, in bytes.
        #[must_use]
        #[inline]
        pub fn allocated_bytes(&self) -> usize {
            self.stats.allocated.load(Acquire)
        }

        /// Returns the total size of the heap, in bytes. This includes memory
        /// that is currently allocated.
        #[must_use]
        #[inline]
        pub fn total_bytes(&self) -> usize {
            self.heap_size.load(Acquire)
        }

        /// Returns the total number of times an allocation attempt has
        /// succeeded, over the lifetime of this heap.
        #[must_use]
        #[inline]
        pub fn alloc_success_count(&self) -> usize {
            self.stats.alloc_success_count.load(Acquire)
        }

        /// Returns the total number of times an allocation attempt could not be
        /// fulfilled because there was insufficient space, over the lifetime of
        /// this heap.
        #[must_use]
        #[inline]
        pub fn alloc_oom_count(&self) -> usize {
            self.stats.alloc_oom_count.load(Acquire)
        }

        /// Returns the total number of times an allocation has been
        /// deallocated, over the lifetime of this heap.
        #[must_use]
        #[inline]
        pub fn dealloc_count(&self) -> usize {
            self.stats.dealloc_count.load(Acquire)
        }
    }

    impl State {
        /// Returns the current amount of free space in the heap, in bytes.
        ///
        /// This is calculated by subtracting [`self.allocated_bytes`] from
        /// [`self.total_bytes`].
        #[must_use]
        #[inline]
        pub fn free_bytes(&self) -> usize {
            self.total_bytes - self.allocated_bytes
        }

        /// Returns the total number of allocation attempts that have been
        /// requested from this heap (successes or failures).
        ///
        /// This is the sum of [`self.alloc_success_count`] and
        /// [`self.alloc_oom_count`].
        #[must_use]
        #[inline]
        pub fn alloc_attempt_count(&self) -> usize {
            self.alloc_success_count + self.alloc_oom_count
        }

        /// Returns the number of currently "live" allocations at the current
        /// point in time.
        ///
        /// This is calculated by subtracting [`self.dealloc_count`] (the number
        /// of allocations which have been freed) from
        /// [`self.alloc_success_count`] (the total number of allocations).
        #[must_use]
        #[inline]
        pub fn live_alloc_count(&self) -> usize {
            self.alloc_success_count - self.dealloc_count
        }
    }

    impl Stats {
        pub(super) const fn new() -> Self {
            Self {
                allocated: AtomicUsize::new(0),
                alloc_success_count: AtomicUsize::new(0),
                alloc_oom_count: AtomicUsize::new(0),
                dealloc_count: AtomicUsize::new(0),
                allocating: AtomicU16::new(0),
                deallocating: AtomicU16::new(0),
            }
        }
    }

    pub(super) fn start_context(counter: &AtomicU16) -> impl Drop + '_ {
        counter.fetch_add(1, Release);
        DecrementOnDrop(counter)
    }

    struct DecrementOnDrop<'counter>(&'counter AtomicU16);

    impl Drop for DecrementOnDrop<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Release);
        }
    }
}
