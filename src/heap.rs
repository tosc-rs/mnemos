///! Allocation types for the Anachro PC.
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    marker::PhantomData,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicUsize, AtomicU8, Ordering},
    mem::MaybeUninit,
};

use crate::{
    containers::{HeapArray, HeapBox, ArcInner, HeapArc, HeapFixedVec},
    node::{Active, ActiveArr, Node, NodeRef, Recycle},
};

use cordyceps::mpsc_queue::{Links, MpscQueue};
use linked_list_allocator::Heap;
use maitake::wait::WaitQueue;

/// An Anachro Heap item
pub struct AHeap {
    freelist: MpscQueue<Recycle>,
    state: AtomicU8,
    heap: UnsafeCell<Heap>,
    heap_wait: WaitQueue,
    inhibit_alloc: AtomicBool,
    any_frees: AtomicBool,
}

// SAFETY: Safety is checked through the `state` member, which uses
// atomic operations to ensure the data is initialized and exclusively
// accessed.
unsafe impl Sync for AHeap {}

impl AHeap {
    /// The AHeap is initialized, and no `HeapGuard`s are active.
    const INIT_IDLE: u8 = 1;

    /// The AHeap is "locked", and cannot currently be retrieved. In MOST cases
    /// this also means the heap is initialized, except for the brief period of
    /// time while the heap is being initialized.
    const BUSY_LOCKED: u8 = 2;

    /// Construct a thread safe async allocator from a pool of memory.
    ///
    /// Safety: The pool of memory MUST be valid for the 'static lifetime, e.g.
    /// obtained by a leaked buffer, a linker section, or some other mechanism.
    /// Additionally, we must semantically have exclusive access to this region
    /// of memory: There must be no other live references, or pointers to this
    /// region that are dereferenced after this call.
    pub unsafe fn bootstrap(addr: *mut u8, size: usize) -> Result<(NonNull<Self>, HeapGuard), ()> {
        // First, we go all bump-allocator to emplace ourselves within this region
        let mut cursor = addr;
        let end = (addr as usize).checked_add(size).ok_or(())?;
        let mut used = 0;

        let stub_ptr;
        let aheap_ptr;

        // We start with the stub node required for our mpsc queue.
        {
            let stub_layout = Layout::new::<Recycle>();
            let stub_offset = cursor.align_offset(stub_layout.align());
            let stub_size = stub_layout.size();
            used += stub_offset;
            used += stub_size;

            if used > size {
                return Err(());
            }

            cursor = cursor.wrapping_add(stub_offset);
            stub_ptr = cursor.cast::<Recycle>();
            stub_ptr.write(Recycle {
                links: Links::new_stub(),
                node_layout: stub_layout,
            });
            cursor = cursor.add(stub_size);
        }

        // Next we allocate ourselves
        {
            let aheap_layout = Layout::new::<Self>();
            let aheap_offset = cursor.align_offset(aheap_layout.align());
            let aheap_size = aheap_layout.size();
            used += aheap_offset;
            used += aheap_size;

            if used > size {
                return Err(());
            }

            cursor = cursor.wrapping_add(aheap_offset);
            aheap_ptr = cursor.cast::<Self>();

            // Increment the cursor, as we use it for the heap initialization
            cursor = cursor.add(aheap_size);

            let heap = Heap::new(cursor, end - (cursor as usize));

            aheap_ptr.write(Self {
                freelist: MpscQueue::new_with_static_stub(&*stub_ptr),
                state: AtomicU8::new(Self::BUSY_LOCKED),
                heap: UnsafeCell::new(heap),
                heap_wait: WaitQueue::new(),
                inhibit_alloc: AtomicBool::new(false),
                any_frees: AtomicBool::new(false),
            });
        }

        // Everything else is now our allocation space.
        let aheap = NonNull::new_unchecked(aheap_ptr);
        let aheap_ref: &'static AHeap = aheap.as_ref();

        // Creating exclusive access to the inner heap is acceptable, as we
        // have marked ourselves with "BUSY_LOCKED", acting as a mutex.
        let guard = HeapGuard { aheap: aheap_ref };

        // Well that went great, I think!
        Ok((aheap, guard))
    }

    pub(crate) unsafe fn release_node(&'static self, node: NonNull<Recycle>) {
        // Can we immediately lock the allocator, avoiding the free list?
        if let Ok(mut guard) = self.lock() {
            let layout: Layout = (*node.as_ptr()).node_layout;
            guard.get_heap().deallocate(node.cast::<u8>(), layout);
            return;
        }

        // Nope! Stick it in the free list
        let node_ref = NodeRef { node };
        self.freelist.enqueue(node_ref);
    }

    pub fn poll(&'static self) {
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

    pub fn lock(&'static self) -> Result<HeapGuard, u8> {
        self.state.compare_exchange(
            Self::INIT_IDLE,
            Self::BUSY_LOCKED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )?;

        // SAFETY: We are already in the BUSY_LOCKED state, we have exclusive access.
        Ok(HeapGuard { aheap: self })
    }

    pub async fn allocate<T>(&'static self, mut item: T) -> HeapBox<T> {
        loop {
            // Is the heap inhibited?
            if !self.inhibit_alloc.load(Ordering::Acquire) {
                // Can we get an exclusive heap handle?
                if let Ok(mut hg) = self.lock() {
                    // Can we allocate our item?
                    match hg.alloc_box(item) {
                        Ok(hb) => {
                            // Yes! Return our allocated item
                            return hb;
                        }
                        Err(it) => {
                            // Nope, the allocation failed.
                            item = it;
                        }
                    }
                }
                // We weren't inhibited before, but something failed. Inhibit
                // further allocations to prevent starving waiting allocations
                self.inhibit_alloc.store(true, Ordering::Release);
            }

            // Didn't succeed, wait until we've done some de-allocations
            self.heap_wait.wait().await.unwrap();
        }
    }

    pub async fn allocate_arc<T>(&'static self, mut item: T) -> HeapArc<T> {
        loop {
            // Is the heap inhibited?
            if !self.inhibit_alloc.load(Ordering::Acquire) {
                // Can we get an exclusive heap handle?
                if let Ok(mut hg) = self.lock() {
                    // Can we allocate our item?
                    match hg.alloc_arc(item) {
                        Ok(hb) => {
                            // Yes! Return our allocated item
                            return hb;
                        }
                        Err(it) => {
                            // Nope, the allocation failed.
                            item = it;
                        }
                    }
                }
                // We weren't inhibited before, but something failed. Inhibit
                // further allocations to prevent starving waiting allocations
                self.inhibit_alloc.store(true, Ordering::Release);
            }

            // Didn't succeed, wait until we've done some de-allocations
            self.heap_wait.wait().await.unwrap();
        }
    }

    pub async fn allocate_array_with<F, T>(&'static self, f: F, count: usize) -> HeapArray<T>
    where
        F: Fn() -> T,
    {
        loop {
            // Is the heap inhibited?
            if !self.inhibit_alloc.load(Ordering::Acquire) {
                // Can we get an exclusive heap handle?
                if let Ok(mut hg) = self.lock() {
                    // Can we allocate our item?
                    //
                    // NOTE: We borrow `f`, since `&Fn() -> T` still impls `Fn() -> T`, and allows
                    // us to potentially call it multiple times.
                    match hg.alloc_box_array_with(&f, count) {
                        Ok(hb) => {
                            // Yes! Return our allocated item
                            return hb;
                        }
                        Err(_) => {
                            // Nope, the allocation failed.
                        }
                    }
                }
                // We weren't inhibited before, but something failed. Inhibit
                // further allocations to prevent starving waiting allocations
                self.inhibit_alloc.store(true, Ordering::Release);
            }

            // Didn't succeed, wait until we've done some de-allocations
            self.heap_wait.wait().await.unwrap();
        }
    }
}

/// A guard type that provides mutually exclusive access to the allocator as
/// long as the guard is held.
pub struct HeapGuard {
    aheap: &'static AHeap,
}

// Public HeapGuard methods
impl HeapGuard {
    fn get_heap(&mut self) -> &mut Heap {
        unsafe { &mut *self.aheap.heap.get() }
    }

    fn clean_allocs(&mut self) {
        let mut any = false;
        // Then, free all pending memory in order to maximize space available.
        let free_list = &self.aheap.freelist;
        let heap = self.get_heap();

        while let Some(node_ref) = free_list.dequeue() {
            // defmt::println!("[ALLOC] FREE: {=usize}", layout.size());
            // SAFETY: We have mutually exclusive access to the allocator, and
            // the pointer and layout are correctly calculated by the relevant
            // FreeBox types.

            let layout = unsafe { node_ref.node.as_ref().node_layout };
            let ptr = node_ref.node.cast::<u8>();

            unsafe {
                heap.deallocate(ptr, layout);
                any = true;
            }
        }

        if any {
            self.aheap.any_frees.store(true, Ordering::Relaxed);
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
        let nnu8 = match self.get_heap().allocate_first_fit(Layout::new::<Node<T>>()) {
            Ok(t) => t,
            Err(_) => return Err(data),
        };
        let nn = nnu8.cast::<Active<T>>();

        // And initialize it with the contents given to us
        unsafe {
            Active::<T>::write_heap(nn, self.aheap);
            Active::<T>::data(nn).as_ptr().write(data);
        }

        Ok(HeapBox {
            ptr: nn,
            pd: PhantomData,
        })
    }

    /// Attempt to allocate a HeapArc using the allocator
    ///
    /// If space was available, the allocation will be returned. If not, an
    /// error will be returned
    pub fn alloc_arc<T>(&mut self, data: T) -> Result<HeapArc<T>, T> {
        // Clean up any pending allocs
        self.clean_allocs();

        // Then, attempt to allocate the requested T.
        let nnu8 = match self.get_heap().allocate_first_fit(Layout::new::<Node<ArcInner<T>>>()) {
            Ok(t) => t,
            Err(_) => return Err(data),
        };
        let nn = nnu8.cast::<Active<ArcInner<T>>>();

        // And initialize it with the contents given to us
        unsafe {
            Active::<ArcInner<T>>::write_heap(nn, self.aheap);
            Active::<ArcInner<T>>::data(nn).as_ptr().write(ArcInner { data, refcnt: AtomicUsize::new(1) });
        }

        Ok(HeapArc {
            ptr: nn,
            pd: PhantomData,
        })
    }

    pub fn alloc_box_array_with<T, F>(&mut self, f: F, count: usize) -> Result<HeapArray<T>, ()>
    where
        F: Fn() -> T,
    {
        // Clean up any pending allocs
        self.clean_allocs();

        // Then figure out the layout of the requested array. This call fails
        // if the total size exceeds ISIZE_MAX, which is exceedingly unlikely
        // (unless the caller calculated something wrong)
        let layout = unsafe { ActiveArr::<T>::layout_for_arr(count) };

        // Then, attempt to allocate the requested T.
        let nnu8 = self.get_heap().allocate_first_fit(layout)?;
        let aa_ptr = nnu8.cast::<ActiveArr<T>>();

        // And initialize it with the contents given to us
        unsafe {
            ActiveArr::<T>::write_heap(aa_ptr, self.aheap);
            ActiveArr::<T>::write_capacity(aa_ptr, count);
            let (start, count) = ActiveArr::<T>::data(aa_ptr);
            let start = start.as_ptr();
            for i in 0..count {
                start.add(i).write((f)());
            }
        }

        Ok(HeapArray {
            ptr: aa_ptr,
            pd: PhantomData,
        })
    }

    pub fn alloc_fixed_vec<T>(&mut self, capacity: usize) -> Result<HeapFixedVec<T>, ()>
    {
        // Clean up any pending allocs
        self.clean_allocs();

        // Then figure out the layout of the requested array. This call fails
        // if the total size exceeds ISIZE_MAX, which is exceedingly unlikely
        // (unless the caller calculated something wrong)
        let layout = unsafe { ActiveArr::<MaybeUninit<T>>::layout_for_arr(capacity) };

        // Then, attempt to allocate the requested T.
        let nnu8 = self.get_heap().allocate_first_fit(layout)?;
        let aa_ptr = nnu8.cast::<ActiveArr<MaybeUninit<T>>>();

        // And initialize it with the contents given to us
        unsafe {
            ActiveArr::<MaybeUninit<T>>::write_heap(aa_ptr, self.aheap);
            ActiveArr::<MaybeUninit<T>>::write_capacity(aa_ptr, capacity);
            let (start, count) = ActiveArr::<MaybeUninit<T>>::data(aa_ptr);
            let start = start.as_ptr();
            for i in 0..count {
                start.add(i).write(MaybeUninit::uninit());
            }
        }

        Ok(HeapFixedVec {
            ptr: aa_ptr,
            pd: PhantomData,
            len: 0,
        })
    }
}

impl Drop for HeapGuard {
    fn drop(&mut self) {
        self.aheap.state.store(AHeap::INIT_IDLE, Ordering::SeqCst)
    }
}
