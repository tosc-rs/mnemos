use core::{cell::UnsafeCell, ops::Deref};
use mnemos_alloc::{
    containers::{HeapArc, HeapArray},
    heap::HeapGuard,
};
use spitebuf::MpMcQueue;

use crate::Kernel;

pub struct KChannel<T> {
    q: HeapArc<MpMcQueue<T, sealed::SpiteData<T>>>,
}

pub struct KProducer<T> {
    q: HeapArc<MpMcQueue<T, sealed::SpiteData<T>>>,
}

impl<T> Clone for KProducer<T> {
    fn clone(&self) -> Self {
        KProducer {
            q: self.q.clone(),
        }
    }
}

pub struct KConsumer<T> {
    q: HeapArc<MpMcQueue<T, sealed::SpiteData<T>>>,
}

mod sealed {
    use super::*;

    pub struct SpiteData<T> {
        pub(crate) data: HeapArray<UnsafeCell<spitebuf::Cell<T>>>,
    }

    unsafe impl<T: Sized> spitebuf::Storage<T> for SpiteData<T> {
        fn buf(&self) -> (*const UnsafeCell<spitebuf::Cell<T>>, usize) {
            let ptr = self.data.as_ptr();
            let len = self.data.len();
            (ptr, len)
        }
    }
}

impl<T> Clone for KChannel<T> {
    fn clone(&self) -> Self {
        Self { q: self.q.clone() }
    }
}

impl<T> Deref for KChannel<T> {
    type Target = MpMcQueue<T, sealed::SpiteData<T>>;

    fn deref(&self) -> &Self::Target {
        &self.q
    }
}

impl<T> KChannel<T> {
    pub async fn new_async(kernel: &'static Kernel, count: usize) -> Self {
        let func = || UnsafeCell::new(spitebuf::single_cell::<T>());
        let heap = kernel.heap();

        let ba = heap.allocate_array_with(func, count).await;
        let q = MpMcQueue::new(sealed::SpiteData { data: ba });
        Self {
            q: heap.allocate_arc(q).await,
        }
    }

    pub fn new(guard: &mut HeapGuard, count: usize) -> Self {
        let func = || UnsafeCell::new(spitebuf::single_cell::<T>());

        let ba = guard.alloc_box_array_with(func, count).unwrap();
        let q = MpMcQueue::new(sealed::SpiteData { data: ba });
        Self {
            q: guard.alloc_arc(q).map_err(drop).unwrap(),
        }
    }

    pub fn split(self) -> (KProducer<T>, KConsumer<T>) {
        let q2 = self.q.clone();
        let prod = KProducer { q: self.q };
        let cons = KConsumer { q: q2 };
        (prod, cons)
    }

    pub fn into_consumer(self) -> KConsumer<T> {
        KConsumer { q: self.q }
    }
}

impl<T> KProducer<T> {
    /// Adds an `item` to the end of the queue
    ///
    /// Returns back the `item` if the queue is full
    #[inline(always)]
    pub fn enqueue_sync(&self, item: T) -> Result<(), T> {
        self.q.enqueue_sync(item)
    }

    #[inline(always)]
    pub async fn enqueue_async(&self, item: T) -> Result<(), T> {
        self.q.enqueue_async(item).await
    }
}

impl<T> KConsumer<T> {
    /// Returns the item in the front of the queue, or `None` if the queue is empty
    #[inline(always)]
    pub fn dequeue_sync(&self) -> Option<T> {
        self.q.dequeue_sync()
    }

    #[inline(always)]
    pub async fn dequeue_async(&self) -> Result<T, ()> {
        self.q.dequeue_async().await
    }

    pub fn producer(&self) -> KProducer<T> {
        KProducer { q: self.q.clone() }
    }
}
