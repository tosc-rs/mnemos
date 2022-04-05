/// Allocation types for the Anachro PC.
///
/// NOTE: This module makes STRONG assumptions that the allocator will be a singleton.
/// This is currently fine, but it is not allowed to make multiple instances of the
/// types within.

use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering},
    mem::{forget, size_of, align_of},
};
use heapless::mpmc::MpMcQueue;
use linked_list_allocator::Heap;

pub static HEAP: AHeap = AHeap::new();
static FREE_Q: FreeQueue = FreeQueue::new();

// AHeap storage goes in a specific section
#[link_section=".aheap.STORAGE"]
static HEAP_BUF: HeapStorage = HeapStorage::new();

// Size is roughly ptr + size + align, so about 3 words.
const FREE_Q_LEN: usize = 128;

/// An Anachro Heap item
pub struct AHeap {
    state: AtomicU8,
    heap: UnsafeCell<MaybeUninit<Heap>>,
}

// SAFETY: Safety is checked through the `state` member, which uses
// atomic operations to ensure the data is initialized and exclusively
// accessed.
unsafe impl Sync for AHeap {}

impl AHeap {
    /// The AHeap is uninitialized. This is the default state.
    const UNINIT: u8 = 0;

    /// The AHeap is initialized, and no `HeapGuard`s are active.
    const INIT_IDLE: u8 = 1;

    /// The AHeap is "locked", and cannot currently be retrieved. In MOST cases
    /// this also means the heap is initialized, except for the brief period of
    /// time while the heap is being initialized.
    const BUSY_LOCKED: u8 = 2;

    /// Create a new, uninitialized AHeap
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(Self::UNINIT),
            heap: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Initialize the AHeap.
    ///
    /// This takes care of initializing all contained memory and tracking variables.
    /// This function should only be called once, and should be called prior to using
    /// the AHeap.
    ///
    /// Returns `Ok(())` if initialization was successful. Returns `Err(())` if the
    /// AHeap was previously initialized.
    pub fn init(&self) -> Result<(), ()> {
        self.state
            .compare_exchange(
                Self::UNINIT,
                Self::BUSY_LOCKED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(drop)?;

        unsafe {
            // Create a heap type from the given storage buffer
            let heap = HEAP_BUF.take_heap();

            // Initialize the Free Queue
            FREE_Q.init();

            // Initialize the heap
            (*self.heap.get()).write(heap);
        }

        // We have exclusive access, a "store" is okay.
        self.state.store(Self::INIT_IDLE, Ordering::SeqCst);

        Ok(())
    }

    pub fn try_lock(&'static self) -> Option<HeapGuard> {
        // The heap must be idle
        self.state
            .compare_exchange(
                Self::INIT_IDLE,
                Self::BUSY_LOCKED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .ok()?;

        // SAFETY: If we were in the INIT_IDLE state, then the heap has been
        // initialized (is valid), and no other access exists (mutually exclusive).
        unsafe {
            let heap = &mut *self.heap.get().cast();
            Some(HeapGuard { heap })
        }
    }
}

struct FreeQueue {
    // NOTE: This is because MpMcQueue has non-zero initialized state, which means
    // it would reside in .data instead of .bss. This moves initialization to runtime,
    // and means that no .data initializer is required
    q: UnsafeCell<MaybeUninit<MpMcQueue<FreeBox, FREE_Q_LEN>>>,
}

// SAFETY: Access to the FreeQueue (a singleton) is mediated by the AHeap.state
// tracking variable.
unsafe impl Sync for FreeQueue {}

impl FreeQueue {
    /// Create a new, uninitialized FreeQueue
    const fn new() -> Self {
        Self {
            q: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Initialize the FreeQueue.
    ///
    /// SAFETY: This function should only ever be called once (usually in the initialization
    /// of the AHeap singleton).
    unsafe fn init(&self) {
        let new = MpMcQueue::new();
        self.q
            .get()
            .cast::<MpMcQueue<FreeBox, FREE_Q_LEN>>()
            .write(new);
    }

    /// Obtain a reference the FreeQueue.
    ///
    /// SAFETY: The free queue MUST have been previously initialized.
    unsafe fn get_unchecked(&self) -> &MpMcQueue<FreeBox, FREE_Q_LEN> {
        // SAFETY: The MpMcQueue type is Sync, so mutual exclusion is not required
        // If the HEAP type has been initialized, so has the FreeQueue singleton,
        // so access is valid.
        (*self.q.get()).assume_init_ref()
    }
}

/// A storage wrapper type for the heap payload.
///
/// The wrapper is required to impl the `Sync` trait
struct HeapStorage {
    data: UnsafeCell<[u8; Self::SIZE_BYTES]>,
}

// SAFETY: Access is only provided through raw pointers, and is exclusively accessed
// through AHeap allocation methods.
unsafe impl Sync for HeapStorage {}

/// An Anachro Heap Box Type
pub struct HeapBox<T> {
    ptr: *mut T,
}

impl<T> Deref for HeapBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr }
    }
}

impl<T> DerefMut for HeapBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.ptr }
    }
}

impl<T> HeapBox<T> {
    /// Create a free_box, with location and layout information necessary
    /// to free the box.
    ///
    /// SAFETY: This function creates aliasing pointers for the allocation. It
    /// should ONLY be called in the destructor of the HeapBox when deallocation
    /// is about to occur, and access to the Box will not be allowed again.
    unsafe fn free_box(&mut self) -> FreeBox {
        FreeBox {
            ptr: NonNull::new_unchecked(self.ptr.cast::<u8>()),
            layout: Layout::new::<T>(),
        }
    }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> &'static mut T {
        let mutref = unsafe { &mut *self.ptr };
        forget(self);
        mutref
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        // Calculate the pointer, size, and layout of this allocation
        let free_box = unsafe { self.free_box() };
        free_box.box_drop();
    }
}

/// An Anachro Heap Array Type
pub struct HeapArray<T> {
    count: usize,
    ptr: *mut T,
}

impl<T> Deref for HeapArray<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        unsafe { core::slice::from_raw_parts(self.ptr, self.count) }
    }
}

impl<T> DerefMut for HeapArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.count) }
    }
}

impl<T> HeapArray<T> {
    /// Create a free_box, with location and layout information necessary
    /// to free the box.
    ///
    /// SAFETY: This function creates aliasing pointers for the allocation. It
    /// should ONLY be called in the destructor of the HeapBox when deallocation
    /// is about to occur, and access to the Box will not be allowed again.
    unsafe fn free_box(&mut self) -> FreeBox {
        // SAFETY: If we allocated this item, it must have been small enough
        // to properly construct a layout. Avoid Layout::array, as it only
        // offers a checked method.
        let layout = unsafe {
            let array_size = size_of::<T>() * self.count;
            Layout::from_size_align_unchecked(array_size, align_of::<T>())
        };
        FreeBox {
            ptr: NonNull::new_unchecked(self.ptr.cast::<u8>()),
            layout: Layout::new::<T>(),
        }
    }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> &'static mut [T] {
        let mutref = unsafe { core::slice::from_raw_parts_mut(self.ptr, self.count) };
        forget(self);
        mutref
    }
}

impl<T> Drop for HeapArray<T> {
    fn drop(&mut self) {
        // Calculate the pointer, size, and layout of this allocation
        let free_box = unsafe { self.free_box() };
        free_box.box_drop();
    }
}

/// A type representing a request to free a given allocation of memory.
struct FreeBox {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl FreeBox {
    fn box_drop(self) {
        // Attempt to get exclusive access to the heap
        if let Some(mut h) = HEAP.try_lock() {
            // If we can access the heap directly, then immediately free this memory
            unsafe {
                h.deref_mut().deallocate(self.ptr, self.layout);
            }
        } else {
            // If not, try to store the allocation into the free list, and it will be
            // reclaimed before the next alloc.
            //
            // SAFETY: A HeapBox can only be created if the Heap, and by extension the
            // FreeQueue, has been previously initialized
            let free_q = unsafe { FREE_Q.get_unchecked() };

            // If the free list is completely full, for now, just panic.
            defmt::unwrap!(free_q.enqueue(self).map_err(drop), "Free list is full!");
        }
    }
}

/// A guard type that provides mutually exclusive access to the allocator as
/// long as the guard is held.
pub struct HeapGuard {
    heap: &'static mut AHeap,
}

// Public HeapGuard methods
impl HeapGuard {
    /// The free space (in bytes) available to the allocator
    pub fn free_space(&self) -> usize {
        self.deref().free()
    }

    /// The used space (in bytes) available to the allocator
    pub fn used_space(&self) -> usize {
        self.deref().used()
    }

    fn clean_allocs(&mut self) {
        // First, grab the Free Queue.
        //
        // SAFETY: A HeapGuard can only be created if the Heap, and by extension the
        // FreeQueue, has been previously initialized
        let free_q = unsafe { FREE_Q.get_unchecked() };

        // Then, free all pending memory in order to maximize space available.
        while let Some(FreeBox { ptr, layout }) = free_q.dequeue() {
            // SAFETY: We have mutually exclusive access to the allocator, and
            // the pointer and layout are correctly calculated by the relevant
            // FreeBox types.
            unsafe {
                self.deref_mut().deallocate(ptr, layout);
            }
        }
    }

    /// Attempt to allocate a HeapBox using the allocator
    ///
    /// If space was available, the allocation will be returned. If not, an
    /// error will be returned
    pub fn alloc_box<T>(&mut self, data: T) -> Result<HeapBox<T>, ()> {
        // Clean up any pending allocs
        self.clean_allocs();

        // Then, attempt to allocate the requested T.
        let nnu8 = self.deref_mut().allocate_first_fit(Layout::new::<T>())?;
        let ptr = nnu8.as_ptr().cast::<T>();

        // And initialize it with the contents given to us
        unsafe {
            ptr.write(data);
        }

        Ok(HeapBox { ptr })
    }

    /// Attempt to allocate a HeapArray using the allocator
    ///
    /// If space was available, the allocation will be returned. If not, an
    /// error will be returned
    pub fn alloc_box_array<T: Copy + ?Sized>(&mut self, data: T, count: usize) -> Result<HeapArray<T>, ()> {
        // Clean up any pending allocs
        self.clean_allocs();

        // Then figure out the layout of the requested array. This call fails
        // if the total size exceeds ISIZE_MAX, which is exceedingly unlikely
        // (unless the caller calculated something wrong)
        let layout = Layout::array::<T>(count).map_err(drop)?;

        // Then, attempt to allocate the requested T.
        let nnu8 = self.deref_mut().allocate_first_fit(layout)?;
        let ptr = nnu8.as_ptr().cast::<T>();

        // And initialize it with the contents given to us
        unsafe {
            for i in 0..count {
                ptr.add(i).write(data);
            }
        }

        Ok(HeapArray { ptr, count })
    }
}

// Private HeapGuard methods.
//
// NOTE: These are NOT impls of the Deref/DerefMut traits, as I don't actually
// want those methods to be available to downstream users of the HeapGuard
// type. For now, I'd like them to only use the "public" allocation interfaces.
impl HeapGuard {
    fn deref(&self) -> &Heap {
        // SAFETY: If we have a HeapGuard, we have single access.
        unsafe { (*self.heap.heap.get()).assume_init_ref() }
    }

    fn deref_mut(&mut self) -> &mut Heap {
        // SAFETY: If we have a HeapGuard, we have single access.
        unsafe { (*self.heap.heap.get()).assume_init_mut() }
    }
}

impl Drop for HeapGuard {
    fn drop(&mut self) {
        // A HeapGuard represents exclusive access to the AHeap. Because of
        // this, a regular store is okay.
        self.heap.state.store(AHeap::INIT_IDLE, Ordering::SeqCst);
    }
}

impl HeapStorage {
    const SIZE_KB: usize = 64;
    const SIZE_BYTES: usize = Self::SIZE_KB * 1024;

    /// Create a new uninitialized storage buffer.
    const fn new() -> Self {
        Self {
            data: UnsafeCell::new([0u8; Self::SIZE_BYTES]),
        }
    }

    /// Obtain the starting address and total size of the storage buffer.
    fn addr_sz(&self) -> (usize, usize) {
        let ptr = self.data.get();
        let addr = ptr as usize;
        (addr, Self::SIZE_BYTES)
    }

    /// Create a Heap object, using the storage contents as the heap memory range.
    ///
    /// SAFETY: This method should only be called once, typically in the
    /// initialization of the AHeap object.
    unsafe fn take_heap(&self) -> Heap {
        let mut heap = Heap::empty();
        let (addr, size) = HEAP_BUF.addr_sz();
        heap.init(addr, size);
        heap
    }
}
