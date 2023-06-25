//! Kernel Channels
//!
//! Kernel Channels are an async/await, MPSC queue, with a fixed backing storage (e.g. they are bounded).

use core::{cell::UnsafeCell, ops::Deref, ptr::NonNull};
use mnemos_alloc::fornow::collections::{Arc, ArrayBuf};
use spitebuf::{DequeueError, EnqueueError, MpScQueue};


/// A Kernel Channel
pub struct KChannel<T> {
    q: Arc<MpScQueue<T, sealed::SpiteData<T>>>,
}

/// A Producer for a [KChannel].
///
/// A `KProducer` can be cloned multiple times, as the backing [KChannel]
/// is an MPSC queue.
pub struct KProducer<T> {
    q: Arc<MpScQueue<T, sealed::SpiteData<T>>>,
}

/// A Consumer for a [KChannel].
///
/// Only a single `KConsumer` can exist at a time for each backing [KChannel],
/// as it is an MPSC queue. A `KConsumer` can also be used to create a new
/// [KProducer] instance.
pub struct KConsumer<T> {
    q: Arc<MpScQueue<T, sealed::SpiteData<T>>>,
}

/// A type-erased [KProducer]. This is currently used only for implementing
/// the type-erased driver service registry.
///
/// It contains a VTable of functions necessary for operations while type-erased,
/// namely cloning and dropping.
pub(crate) struct ErasedKProducer {
    erased_q: NonNull<MpScQueue<(), sealed::SpiteData<()>>>,
    dropper: unsafe fn(NonNull<MpScQueue<(), sealed::SpiteData<()>>>),
    cloner: unsafe fn(&Self) -> Self,
}

// KChannel

impl<T> Clone for KChannel<T> {
    fn clone(&self) -> Self {
        Self { q: self.q.clone() }
    }
}

impl<T> Deref for KChannel<T> {
    type Target = MpScQueue<T, sealed::SpiteData<T>>;

    fn deref(&self) -> &Self::Target {
        &self.q
    }
}

impl<T> KChannel<T> {
    /// Create a new `KChannel<T>` with room for `count` elements on the given
    /// Kernel's allocator.
    pub async fn new_async(count: usize) -> Self {
        let ba = ArrayBuf::new_uninit(count).await;
        let q = MpScQueue::new(sealed::SpiteData { data: ba });
        Self {
            q: Arc::new(q).await,
        }
    }

    /// Create a new `KChannel<T>` with room for `count` elements on the given
    /// Kernel's allocator. Used for pre-async initialization steps
    pub fn new(count: usize) -> Self {
        let ba = ArrayBuf::try_new_uninit(count).unwrap();
        let q = MpScQueue::new(sealed::SpiteData { data: ba });
        Self {
            q: Arc::try_new(q).map_err(drop).unwrap(),
        }
    }

    /// Split the KChannel into a pair of [KProducer] and [KConsumer].
    pub fn split(self) -> (KProducer<T>, KConsumer<T>) {
        let q2 = self.q.clone();
        let prod = KProducer { q: self.q };
        let cons = KConsumer { q: q2 };
        (prod, cons)
    }

    /// Convert the `KChannel` directly into a `KConsumer`
    ///
    /// Because a [KConsumer] can be used to create a [KProducer], this method
    /// is handy when the producer is not immediately needed.
    pub fn into_consumer(self) -> KConsumer<T> {
        KConsumer { q: self.q }
    }
}

// KProducer

impl<T> Clone for KProducer<T> {
    fn clone(&self) -> Self {
        KProducer { q: self.q.clone() }
    }
}

impl<T> KProducer<T> {
    /// Attempt to immediately add an `item` to the end of the queue
    ///
    /// Returns back the `item` if the queue is full
    #[inline(always)]
    pub fn enqueue_sync(&self, item: T) -> Result<(), EnqueueError<T>> {
        self.q.enqueue_sync(item)
    }

    /// Attempt to asynchronously add an `item` to the end of the queue.
    ///
    /// If the queue is full, this method will yield until there is space
    /// available.
    #[inline(always)]
    pub async fn enqueue_async(&self, item: T) -> Result<(), EnqueueError<T>> {
        self.q.enqueue_async(item).await
    }

    pub(crate) fn type_erase(self) -> ErasedKProducer {
        let typed_q: NonNull<MpScQueue<T, sealed::SpiteData<T>>> = Arc::into_raw(self.q);
        let erased_q: NonNull<MpScQueue<(), sealed::SpiteData<()>>> = typed_q.cast();

        ErasedKProducer {
            erased_q,
            dropper: ErasedKProducer::drop_erased::<T>,
            cloner: ErasedKProducer::clone_erased::<T>,
        }
    }
}

// KConsumer

impl<T> KConsumer<T> {
    /// Immediately returns the item in the front of the queue, or
    /// `None` if the queue is empty
    #[inline(always)]
    pub fn dequeue_sync(&self) -> Option<T> {
        self.q.dequeue_sync()
    }

    /// Await the availability of an item from the front of the queue.
    ///
    /// If no item is available, this function will yield until an item
    /// has been enqueued
    #[inline(always)]
    pub async fn dequeue_async(&self) -> Result<T, DequeueError> {
        self.q.dequeue_async().await
    }

    /// Create a [KProducer] for this KConsumer (and its backing [KChannel]).
    pub fn producer(&self) -> KProducer<T> {
        KProducer { q: self.q.clone() }
    }
}

// ErasedKProducer

impl Clone for ErasedKProducer {
    fn clone(&self) -> Self {
        unsafe { (self.cloner)(self) }
    }
}

impl ErasedKProducer {
    /// Clone the ErasedKProducer. The resulting ErasedKProducer will be for the same
    /// underlying [KChannel] and type.
    pub(crate) fn clone_erased<T>(&self) -> Self {
        let typed_q: NonNull<MpScQueue<T, sealed::SpiteData<T>>> = self.erased_q.cast();
        unsafe {
            Arc::increment_strong_count(typed_q.as_ptr());
        }

        Self {
            erased_q: self.erased_q,
            dropper: self.dropper,
            cloner: self.cloner,
        }
    }

    /// Clone the ErasedKProducer, while also re-typing to the unleaked [KProducer] type.
    ///
    /// SAFETY:
    ///
    /// The type `T` MUST be the same `T` that was used to create this ErasedKProducer,
    /// otherwise undefined behavior will occur.
    pub(crate) unsafe fn clone_typed<T>(&self) -> KProducer<T> {
        let typed_q: NonNull<MpScQueue<T, sealed::SpiteData<T>>> = self.erased_q.cast();
        let q = unsafe {
            Arc::increment_strong_count(typed_q.as_ptr());
            Arc::from_raw(typed_q)
        };

        KProducer { q }
    }

    /// Drop the ErasedKProducer, while also re-typing the leaked [KProducer] type.
    ///
    /// SAFETY:
    ///
    /// The type `T` MUST be the same `T` that was used to create this ErasedKProducer,
    /// otherwise undefined behavior will occur.
    pub(crate) unsafe fn drop_erased<T>(ptr: NonNull<MpScQueue<(), sealed::SpiteData<()>>>) {
        let ptr = ptr.cast::<MpScQueue<T, sealed::SpiteData<T>>>();
        let _ = Arc::from_raw(ptr);
    }
}

impl Drop for ErasedKProducer {
    fn drop(&mut self) {
        unsafe {
            (self.dropper)(self.erased_q);
        }
    }
}

pub(crate) mod sealed {
    use mnemos_alloc::fornow::collections::ArrayBuf;

    use super::*;

    pub struct SpiteData<T> {
        pub(crate) data: ArrayBuf<spitebuf::Cell<T>>,
    }

    unsafe impl<T: Sized> spitebuf::Storage<T> for SpiteData<T> {
        fn buf(&self) -> (*const UnsafeCell<spitebuf::Cell<T>>, usize) {
            let (ptr, len) = self.data.ptrlen();
            (ptr.as_ptr().cast(), len)
        }
    }
}
