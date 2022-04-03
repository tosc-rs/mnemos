use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering},
};
use heapless::mpmc::MpMcQueue;
use linked_list_allocator::Heap;

pub static HEAP: AHeap = AHeap::new();

static HEAP_BUF: HeapStorage = HeapStorage::new();
static FREE_Q: FreeQueue = FreeQueue::new();
const FREE_Q_LEN: usize = 128;

pub struct AHeap {
    state: AtomicU8,
    heap: UnsafeCell<MaybeUninit<Heap>>,
}

unsafe impl Sync for AHeap {}

impl AHeap {
    const UNINIT: u8 = 0;
    const INIT_IDLE: u8 = 1;
    const INIT_BUSY: u8 = 2;

    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(Self::UNINIT),
            heap: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn init(&self) -> Result<(), ()> {
        self.state
            .compare_exchange(
                Self::UNINIT,
                Self::INIT_BUSY,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(drop)?;

        unsafe {
            let heap = HEAP_BUF.init();

            // Initialize the Free Queue
            FREE_Q.init();

            // Initialize the heap
            (*self.heap.get()).write(heap);
        }

        self.state.store(Self::INIT_IDLE, Ordering::SeqCst);

        Ok(())
    }

    pub fn try_lock(&'static self) -> Option<HeapGuard> {
        self.state
            .compare_exchange(
                Self::INIT_IDLE,
                Self::INIT_BUSY,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .ok()?;

        unsafe {
            let heap = &mut *self.heap.get().cast();
            Some(HeapGuard { heap })
        }
    }
}

struct FreeQueue {
    q: UnsafeCell<MaybeUninit<MpMcQueue<FreeBox, FREE_Q_LEN>>>,
}

unsafe impl Sync for FreeQueue {}

impl FreeQueue {
    const fn new() -> Self {
        Self {
            q: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    unsafe fn init(&self) {
        let new = MpMcQueue::new();
        self.q
            .get()
            .cast::<MpMcQueue<FreeBox, FREE_Q_LEN>>()
            .write(new);
    }

    fn try_get(&self) -> Result<&MpMcQueue<FreeBox, FREE_Q_LEN>, ()> {
        let state = HEAP.state.load(Ordering::SeqCst);

        if state == AHeap::UNINIT {
            Err(())
        } else {
            unsafe { Ok((*self.q.get()).assume_init_ref()) }
        }
    }
}

struct HeapStorage {
    data: UnsafeCell<[u8; Self::SIZE_BYTES]>,
}

unsafe impl Sync for HeapStorage {}

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
    unsafe fn free_box(&mut self) -> FreeBox {
        FreeBox {
            ptr: NonNull::new_unchecked(self.ptr.cast::<u8>()),
            layout: Layout::new::<T>(),
        }
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        let free_box = unsafe { self.free_box() };

        if let Some(mut h) = HEAP.try_lock() {
            // If we can access the heap directly, then immediately free this memory
            unsafe {
                h.deallocate(free_box.ptr, free_box.layout);
            }
        } else {
            // If not, try to store the allocation into the free list, and it will be
            // reclaimed before the next alloc.
            let free_q = defmt::unwrap!(FREE_Q.try_get());

            // If the free list is completely full, for now, just panic.
            defmt::unwrap!(free_q.enqueue(free_box).map_err(drop), "Free list is full!");
        }
    }
}

pub struct FreeBox {
    ptr: NonNull<u8>,
    layout: Layout,
}

pub struct HeapGuard {
    heap: &'static mut AHeap,
}

impl HeapGuard {
    pub fn alloc_box<T>(&mut self, data: T) -> Result<HeapBox<T>, ()> {
        // First, free all pending memory
        let free_q = FREE_Q.try_get()?;
        while let Some(FreeBox { ptr, layout }) = free_q.dequeue() {
            unsafe {
                self.deallocate(ptr, layout);
            }
        }

        // Then, attempt to allocate the requested T.
        let nnu8 = self.allocate_first_fit(Layout::new::<T>())?;
        let ptr = nnu8.as_ptr().cast::<T>();

        unsafe {
            ptr.write(data);
        }

        Ok(HeapBox { ptr })
    }
}

impl Deref for HeapGuard {
    type Target = Heap;

    fn deref(&self) -> &Self::Target {
        // SAFETY: If we have a HeapGuard, we have single access.
        unsafe { (*self.heap.heap.get()).assume_init_ref() }
    }
}

impl DerefMut for HeapGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: If we have a HeapGuard, we have single access.
        unsafe { (*self.heap.heap.get()).assume_init_mut() }
    }
}

impl Drop for HeapGuard {
    fn drop(&mut self) {
        self.heap.state.store(AHeap::INIT_IDLE, Ordering::SeqCst);
    }
}

impl HeapStorage {
    const SIZE_KB: usize = 64;
    const SIZE_BYTES: usize = Self::SIZE_KB * 1024;

    const fn new() -> Self {
        Self {
            data: UnsafeCell::new([0u8; Self::SIZE_BYTES]),
        }
    }

    fn addr_sz(&self) -> (usize, usize) {
        let ptr = self.data.get();
        let addr = ptr as usize;
        (addr, Self::SIZE_BYTES)
    }

    // SAFETY: Only call once!
    unsafe fn init(&self) -> Heap {
        let mut heap = Heap::empty();
        let (addr, size) = HEAP_BUF.addr_sz();
        heap.init(addr, size);
        heap
    }
}
