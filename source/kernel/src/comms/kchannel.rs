use core::{cell::UnsafeCell, ops::Deref};
use mnemos_alloc::{
    containers::{HeapArc, HeapArray},
    heap::HeapGuard,
};
use spitebuf::MpMcQueue;

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

pub struct KChannel<T> {
    q: HeapArc<MpMcQueue<T, sealed::SpiteData<T>>>,
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
    pub fn new(guard: &mut HeapGuard, count: usize) -> Self {
        let func = || UnsafeCell::new(spitebuf::single_cell::<T>());

        let ba = guard.alloc_box_array_with(func, count).unwrap();
        let q = MpMcQueue::new(sealed::SpiteData { data: ba });
        Self {
            q: guard.alloc_arc(q).map_err(drop).unwrap(),
        }
    }
}
