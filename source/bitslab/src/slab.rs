use crate::{
    index::IndexAlloc64,
    loom::{
        cell::{MutPtr, UnsafeCell},
        sync::atomic::{AtomicU64, Ordering},
    },
};
use core::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};
use maitake_sync::WaitQueue;

pub struct Bitslab64<T> {
    alloc: IndexAlloc64,
    slab: [UnsafeCell<MaybeUninit<T>>; 64],
    initialized: AtomicU64,
    initializer: fn() -> T,
    free_wait: WaitQueue,
}

impl<T: Default> Bitslab64<T> {
    #[cfg(not(all(test, loom)))]
    #[must_use]
    pub const fn new() -> Self {
        Self::with_initializer(T::default)
    }

    #[cfg(all(test, loom))]
    #[must_use]
    pub fn new() -> Self {
        Self::with_initializer(T::default)
    }
}

#[must_use = "a `RefMut` does nothing if not dereferenced"]
pub struct RefMut<'slab, T> {
    value: MutPtr<MaybeUninit<T>>,
    _free: FreeOnDrop<'slab, T>,
}

struct FreeOnDrop<'slab, T> {
    slab: &'slab Bitslab64<T>,
    idx: u8,
}

// Macro for initializing arrays with non-`Copy` initializers.
// Based on https://stackoverflow.com/a/36259524
//
// TODO(eliza): Maybe this should go in a "utils" crate eventually?
macro_rules! array {
    (@accum (0, $($_es:expr),*) -> ($($body:tt)*))
        => {array!(@as_expr [$($body)*])};
    (@accum (1, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (0, $($es),*) -> ($($body)* $($es,)*))};
    (@accum (2, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (0, $($es),*) -> ($($body)* $($es,)* $($es,)*))};
    (@accum (3, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (2, $($es),*) -> ($($body)* $($es,)*))};
    (@accum (4, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (2, $($es,)* $($es),*) -> ($($body)*))};
    (@accum (5, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (4, $($es),*) -> ($($body)* $($es,)*))};
    (@accum (6, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (4, $($es),*) -> ($($body)* $($es,)* $($es,)*))};
    (@accum (7, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (4, $($es),*) -> ($($body)* $($es,)* $($es,)* $($es,)*))};
    (@accum (8, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (4, $($es,)* $($es),*) -> ($($body)*))};
    (@accum (16, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (8, $($es,)* $($es),*) -> ($($body)*))};
    (@accum (32, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (16, $($es,)* $($es),*) -> ($($body)*))};
    (@accum (64, $($es:expr),*) -> ($($body:tt)*))
        => {array!(@accum (32, $($es,)* $($es),*) -> ($($body)*))};

    (@as_expr $e:expr) => {$e};

    [$e:expr; $n:tt] => { array!(@accum ($n, $e) -> ()) };
}

impl<T> Bitslab64<T> {
    pub const CAPACITY: usize = 64;

    #[cfg(not(all(test, loom)))]
    #[must_use]
    pub const fn with_initializer(initializer: fn() -> T) -> Self {
        Self {
            alloc: IndexAlloc64::new(),
            slab: array![UnsafeCell::new(MaybeUninit::uninit()); 64],
            initialized: AtomicU64::new(0),
            initializer,
            free_wait: WaitQueue::new(),
        }
    }

    #[cfg(all(test, loom))]
    #[must_use]
    pub fn with_initializer(initializer: fn() -> T) -> Self {
        Self {
            alloc: IndexAlloc64::new(),
            slab: array![UnsafeCell::new(MaybeUninit::uninit()); 64],
            initialized: AtomicU64::new(0),
            initializer,
            free_wait: WaitQueue::new(),
        }
    }

    pub async fn alloc(&self) -> RefMut<'_, T> {
        loop {
            #[cfg(test)]
            tracing::debug!("try allocate...");
            if let Some(a) = self.try_alloc() {
                #[cfg(test)]
                tracing::debug!("try allocate -> success");
                return a;
            }

            #[cfg(test)]
            tracing::debug!("try allocate -> fail");

            self.free_wait
                .wait()
                .await
                .expect("Bitslab64 WaitQueues are never closed!");

            #[cfg(test)]
            tracing::debug!("try allocate -> fail -> woken");
        }
    }

    pub fn try_alloc(&self) -> Option<RefMut<'_, T>> {
        let idx = self.alloc.allocate()?;
        let should_init = {
            let mask = 1 << idx as u64;
            let bitfield = self.initialized.fetch_or(mask, Ordering::AcqRel);
            bitfield & mask == 0
        };
        let value = self.slab[idx as usize].get_mut();
        if should_init {
            unsafe {
                // Safety: we claimed exclusive ownership over this index by
                // allocating it from the index allocator.
                value.deref().write((self.initializer)());
            }
        }

        Some(RefMut {
            value,
            _free: FreeOnDrop { slab: self, idx },
        })
    }

    unsafe fn free(&self, idx: u8) {
        #[cfg(test)]
        tracing::debug!(idx, "free");

        self.alloc.free(idx);
        self.free_wait.wake();
    }
}

// === impl RefMut ===

impl<T> Deref for RefMut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe {
            // Safety: we're about to do two unsafe things: dereferencing an
            // `UnsafeCell` `MutPtr`, and assuming a `MaybeUninit` is
            // initialized.
            //
            // It's safe to call `value.deref()` here, because we only construct
            // a `RefMut` after having claimed exclusive access to the index
            // from the index allocator.
            //
            // Similarly, the call to `assume_init_ref()` is okay, because we
            // only construct a `RefMut` after ensuring that the value has been
            // initialized.
            self.value.deref().assume_init_ref()
        }
    }
}

impl<T> DerefMut for RefMut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            // Safety: we're about to do two unsafe things: dereferencing an
            // `UnsafeCell` `MutPtr`, and assuming a `MaybeUninit` is
            // initialized.
            //
            // It's safe to call `value.deref()` here, because we only construct
            // a `RefMut` after having claimed exclusive access to the index
            // from the index allocator.
            //
            // Similarly, the call to `assume_init_ref()` is okay, because we
            // only construct a `RefMut` after ensuring that the value has been
            // initialized.
            self.value.deref().assume_init_mut()
        }
    }
}

impl<T> Drop for FreeOnDrop<'_, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            self.slab.free(self.idx);
        }
    }
}

unsafe impl<T: Send + Sync> Sync for Bitslab64<T> {}
unsafe impl<T: Send + Sync> Send for Bitslab64<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loom::{self, alloc::Track, sync::Arc};
    use tracing::Instrument;

    #[test]
    fn items_dropped() {
        fn run(slab: Arc<Bitslab64<Track<()>>>) -> impl FnOnce() {
            move || {
                let item1 = slab.try_alloc().unwrap();
                let item2 = slab.try_alloc().unwrap();
                drop((item1, item2));
            }
        }

        loom::model(|| {
            let slab = Arc::new(Bitslab64::<Track<()>>::with_initializer(|| Track::new(())));
            loom::thread::spawn(run(slab.clone()));
            run(slab)();
        })
    }

    #[test]
    fn try_alloc_nodrop() {
        fn run(thread: usize, slab: &Arc<Bitslab64<i32>>) -> impl FnOnce() {
            let slab = slab.clone();
            move || {
                let mut guards = Vec::new();
                for i in 0..32 {
                    match slab.try_alloc() {
                        Some(mut item) => {
                            println!("[thread {thread}] allocated item {i}");
                            *item = i;
                            guards.push(item);
                        }
                        None => {
                            panic!("[thread {thread}] failed to allocate item {i}!");
                        }
                    }
                }
            }
        }

        loom::model(|| {
            let slab = Arc::new(Bitslab64::new());
            let t1 = loom::thread::spawn(run(0, &slab));
            run(1, &slab)();
            t1.join().unwrap();
        })
    }

    #[test]
    fn alloc_async_nodrop() {
        fn run(thread: usize, slab: &Arc<Bitslab64<i32>>) -> impl FnOnce() {
            let slab = slab.clone();

            move || {
                loom::future::block_on(async move {
                    let mut guards = Vec::new();
                    for i in 0..32 {
                        let span = tracing::info_span!("alloc", item = i, thread);
                        let mut item = slab.alloc().instrument(span.clone()).await;
                        let _enter = span.enter();
                        tracing::info!("allocated item");
                        *item = i;
                        guards.push(item);
                    }
                })
            }
        }

        loom::model(|| {
            let slab = Arc::new(Bitslab64::new());
            let t1 = loom::thread::spawn(run(0, &slab));
            run(1, &slab)();
            t1.join().unwrap();
        })
    }

    #[test]
    fn alloc_async_drop() {
        loom::model(|| {
            let slab = Arc::new(Bitslab64::new());

            let mut guards = Vec::with_capacity(32);
            loom::future::block_on(async {
                for i in 0..64 {
                    let span = tracing::info_span!("alloc", item = i, thread = 0);
                    let mut item = slab.alloc().instrument(span.clone()).await;
                    let _enter = span.enter();
                    tracing::info!("allocated item");
                    *item = i;
                    guards.push(item);
                }
            });

            let t1 = loom::thread::spawn({
                let slab = slab.clone();
                move || {
                    loom::future::block_on(async move {
                        let mut guards = Vec::with_capacity(32);
                        for i in 0..64 {
                            let span = tracing::info_span!("alloc", item = i, thread = 1);
                            let mut item = slab.alloc().instrument(span.clone()).await;
                            let _enter = span.enter();
                            tracing::info!("allocated item");
                            *item = i;
                            guards.push(item);
                        }
                    })
                }
            });

            for (i, guard) in guards.drain(..).enumerate() {
                let _span = tracing::info_span!("drop", i, thread = 0).entered();
                drop(guard);
                tracing::info!("dropped item");
            }

            t1.join().unwrap();
        })
    }
}
