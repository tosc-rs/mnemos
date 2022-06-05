/// Allocation types for the Anachro PC.
///
/// NOTE: This module makes STRONG assumptions that the allocator will be a singleton.
/// This is currently fine, but it is not allowed to make multiple instances of the
/// types within.
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    mem::{align_of, forget, size_of},
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering, AtomicBool}, pin::Pin,
};
use heapless::mpmc::MpMcQueue;
use linked_list_allocator::Heap;
use maitake::wait::WaitQueue;

pub static HEAP: AHeap = AHeap::new();

// TODO: I could replace the free queue with a cordyceps MpMcQueue for the cost of an extra pointer
// (and maybe layout? maybe not?) per allocation
static FREE_Q: FreeQueue = FreeQueue::new();

// Size is roughly ptr + size + align, so about 3 words.
const FREE_Q_LEN: usize = 128;

/// An Anachro Heap item
pub struct AHeap {
    state: AtomicU8,
    heap: UnsafeCell<MaybeUninit<Heap>>,
    heap_wait: WaitQueue,
    inhibit_alloc: AtomicBool,
    any_frees: AtomicBool,
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
            inhibit_alloc: AtomicBool::new(true),
            heap_wait: WaitQueue::new(),
            any_frees: AtomicBool::new(false),
        }
    }

    pub fn init_exclusive(&self, addr: usize, size: usize) -> Result<HeapGuard, ()> {
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
            let mut heap = Heap::empty();
            heap.init(addr, size);

            // Initialize the Free Queue
            FREE_Q.init();

            // Initialize the heap
            (*self.heap.get()).write(heap);
        }

        self.inhibit_alloc.store(false, Ordering::Release);

        // SAFETY: We are already in the BUSY_LOCKED state, we have exclusive access.
        unsafe {
            let heap = &mut *self.heap.get().cast();
            Ok(HeapGuard { heap })
        }
    }

    pub fn poll(&self) {
        let mut hg = self.lock().unwrap();

        // Clean any pending allocs
        hg.clean_allocs();

        // Did we perform any deallocations?
        if self.any_frees.swap(false, Ordering::SeqCst) {
            // Clear the inhibit flag
            self.inhibit_alloc.store(false, Ordering::SeqCst);

            // Wake any tasks waiting on alloc
            self.heap_wait.wake_all();
        }
    }

    pub fn lock(&self) -> Result<HeapGuard, u8> {
        self.state
            .compare_exchange(
                Self::INIT_IDLE,
                Self::BUSY_LOCKED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )?;

        // SAFETY: We are already in the BUSY_LOCKED state, we have exclusive access.
        unsafe {
            let heap = &mut *self.heap.get().cast();
            Ok(HeapGuard { heap })
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

/// An Anachro Heap Box Type
pub struct HeapBox<T> {
    ptr: *mut T,
}

unsafe impl<T> Send for HeapBox<T> {}

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
    pub unsafe fn from_leaked(ptr: *mut T) -> Self {
        Self { ptr }
    }

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
        let mutref: &'static mut _ = unsafe { &mut *self.ptr };
        forget(self);
        mutref
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        unsafe {
            self.ptr.drop_in_place();
        }
        // Calculate the pointer, size, and layout of this allocation
        let free_box = unsafe { self.free_box() };
        free_box.box_drop();
    }
}

/// An Anachro Heap Array Type
pub struct HeapArray<T> {
    pub(crate) count: usize,
    pub(crate) ptr: *mut T,
}

unsafe impl<T> Send for HeapArray<T> {}

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
    pub unsafe fn from_leaked(ptr: *mut T, count: usize) -> Self {
        Self { ptr, count }
    }

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
        let layout = {
            let array_size = size_of::<T>() * self.count;
            Layout::from_size_align_unchecked(array_size, align_of::<T>())
        };
        FreeBox {
            ptr: NonNull::new_unchecked(self.ptr.cast::<u8>()),
            layout,
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
        for i in 0..self.count {
            unsafe {
                self.ptr.add(i).drop_in_place();
            }
        }
        // Calculate the pointer, size, and layout of this allocation
        let free_box = unsafe { self.free_box() };
        // defmt::println!("[ALLOC] dropping array: {=usize}", free_box.layout.size());
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
        // Attempt to immediately drop, if possible
        if let Ok(mut hg) = HEAP.lock() {
            unsafe {
                hg.free_raw(self.ptr, self.layout);
            }
            return;
        } else {
            // Nope, couldn't lock the heap.
            //
            // Try to store the allocation into the free list, and it will be
            // reclaimed before the next alloc.
            //
            // SAFETY: A HeapBox can only be created if the Heap, and by extension the
            // FreeQueue, has been previously initialized
            let free_q = unsafe { FREE_Q.get_unchecked() };

            // If the free list is completely full, for now, just panic.
            free_q.enqueue(self).map_err(drop).expect("Should have had room in the free list...");
        }
    }
}

/// A guard type that provides mutually exclusive access to the allocator as
/// long as the guard is held.
pub struct HeapGuard {
    heap: &'static mut Heap,
}

// Public HeapGuard methods
impl HeapGuard {
    pub unsafe fn free_raw(&mut self, ptr: NonNull<u8>, layout: Layout) {
        self.deref_mut().deallocate(ptr, layout);
        HEAP.any_frees.store(true, Ordering::Relaxed);
    }

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

        let mut any = false;
        // Then, free all pending memory in order to maximize space available.
        while let Some(FreeBox { ptr, layout }) = free_q.dequeue() {
            // defmt::println!("[ALLOC] FREE: {=usize}", layout.size());
            // SAFETY: We have mutually exclusive access to the allocator, and
            // the pointer and layout are correctly calculated by the relevant
            // FreeBox types.
            unsafe {
                self.deref_mut().deallocate(ptr, layout);
                any = true;
            }
        }

        if any {
            HEAP.any_frees.store(true, Ordering::Relaxed);
        }
    }

    /// Attempt to allocate a HeapBox using the allocator
    ///
    /// If space was available, the allocation will be returned. If not, an
    /// error will be returned
    pub fn alloc_box<T>(&mut self, data: T) -> Result<HeapBox<T>, T> {
        // Clean up any pending allocs
        self.clean_allocs();

        // Then, attempt to allocate the requested T.
        let nnu8 = match self.deref_mut().allocate_first_fit(Layout::new::<T>()) {
            Ok(t) => t,
            Err(_) => return Err(data),
        };
        let ptr = nnu8.as_ptr().cast::<T>();

        // And initialize it with the contents given to us
        unsafe {
            ptr.write(data);
        }

        Ok(HeapBox { ptr })
    }

    pub fn alloc_pin_box<T: Unpin>(&mut self, data: T) -> Result<Pin<HeapBox<T>>, T> {
        Ok(Pin::new(self.alloc_box(data)?))
    }

    pub fn leak_send<T>(&mut self, inp: T) -> Result<&'static mut T, T>
    where
        T: Send + Sized + 'static,
    {
        let boxed = self.alloc_box(inp)?;
        Ok(boxed.leak())
    }

    /// Attempt to allocate a HeapArray using the allocator
    ///
    /// If space was available, the allocation will be returned. If not, an
    /// error will be returned
    pub fn alloc_box_array<T: Copy + ?Sized>(
        &mut self,
        data: T,
        count: usize,
    ) -> Result<HeapArray<T>, ()> {
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
        &*self.heap
    }

    fn deref_mut(&mut self) -> &mut Heap {
        self.heap
    }
}

impl Drop for HeapGuard {
    #[track_caller]
    fn drop(&mut self) {
        // A HeapGuard represents exclusive access to the AHeap. Because of
        // this, a regular store is okay.
        HEAP.state.store(AHeap::INIT_IDLE, Ordering::SeqCst);
    }
}

pub async fn allocate<T>(mut item: T) -> HeapBox<T> {
    loop {
        // Is the heap inhibited?
        if !HEAP.inhibit_alloc.load(Ordering::Acquire) {
            // Can we get an exclusive heap handle?
            if let Ok(mut hg) = HEAP.lock() {
                // Can we allocate our item?
                match hg.alloc_box(item) {
                    Ok(hb) => {
                        // Yes! Return our allocated item
                        return hb;
                    }
                    Err(it) => {
                        // Nope, the allocation failed.
                        item = it;
                    },
                }
            }
            // We weren't inhibited before, but something failed. Inhibit
            // further allocations to prevent starving waiting allocations
            HEAP.inhibit_alloc.store(true, Ordering::Release);
        }

        // Didn't succeed, wait until we've done some de-allocations
        HEAP.heap_wait
            .wait()
            .await
            .unwrap();
    }
}

