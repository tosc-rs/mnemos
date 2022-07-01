use crate::node::{Active, ActiveArr};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{drop_in_place, addr_of};
use core::slice::{from_raw_parts, from_raw_parts_mut};
use core::sync::atomic::{Ordering, AtomicUsize};
use core::{
    mem::forget,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

/// An Anachro Heap Box Type
pub struct HeapBox<T> {
    pub(crate) ptr: NonNull<Active<T>>,
    pub(crate) pd: PhantomData<Active<T>>,
}

pub(crate) struct ArcInner<T> {
    pub(crate) data: T,
    pub(crate) refcnt: AtomicUsize,
}

pub struct HeapArc<T> {
    pub(crate) ptr: NonNull<Active<ArcInner<T>>>,
    pub(crate) pd: PhantomData<Active<ArcInner<T>>>,
}

/// An Anachro Heap Array Type
pub struct HeapArray<T> {
    pub(crate) ptr: NonNull<ActiveArr<T>>,
    pub(crate) pd: PhantomData<Active<T>>,
}

/// An Anachro Heap Array Type
pub struct HeapFixedVec<T> {
    pub(crate) ptr: NonNull<ActiveArr<MaybeUninit<T>>>,
    pub(crate) len: usize,
    pub(crate) pd: PhantomData<Active<MaybeUninit<T>>>,
}

// === impl HeapBox ===

unsafe impl<T> Send for HeapBox<T> {}

impl<T> Deref for HeapBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*Active::<T>::data(self.ptr).as_ptr() }
    }
}

impl<T> DerefMut for HeapBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *Active::<T>::data(self.ptr).as_ptr() }
    }
}

impl<T> HeapBox<T> {
    pub unsafe fn from_leaked(ptr: NonNull<T>) -> Self {
        Self {
            ptr: Active::<T>::from_leaked_ptr(ptr),
            pd: PhantomData,
        }
    }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> NonNull<T> {
        let nn = unsafe { Active::<T>::data(self.ptr) };
        forget(self);
        nn
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        unsafe {
            let item_ptr = Active::<T>::data(self.ptr).as_ptr();
            drop_in_place(item_ptr);
            Active::<T>::yeet(self.ptr);
        }
    }
}

// === impl HeapArc ===

unsafe impl<T> Send for HeapArc<T> {}

impl<T> Deref for HeapArc<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe {
            let aiptr: *mut ArcInner<T> = Active::<ArcInner<T>>::data(self.ptr).as_ptr();
            let dptr: *const T = addr_of!((*aiptr).data);
            &*dptr
        }
    }
}

impl<T> Drop for HeapArc<T> {
    fn drop(&mut self) {
        unsafe {
            let (aiptr, needs_drop) = {
                let aitem_ptr = Active::<ArcInner<T>>::data(self.ptr).as_ptr();
                let old = (*aitem_ptr).refcnt.fetch_sub(1, Ordering::SeqCst);
                debug_assert_ne!(old, 0);
                (aitem_ptr, old == 1)
            };

            if needs_drop {
                drop_in_place(aiptr);
                Active::<ArcInner<T>>::yeet(self.ptr);
            }
        }
    }
}

impl<T> Clone for HeapArc<T> {
    fn clone(&self) -> Self {
        unsafe {
            let aitem_nn = Active::<ArcInner<T>>::data(self.ptr);
            aitem_nn.as_ref().refcnt.fetch_add(1, Ordering::SeqCst);

            HeapArc {
                ptr: self.ptr,
                pd: PhantomData,
            }
        }
    }
}

// === impl HeapArray ===

unsafe impl<T> Send for HeapArray<T> {}

impl<T> Deref for HeapArray<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        unsafe {
            let (nn_ptr, count) = ActiveArr::<T>::data(self.ptr);
            from_raw_parts(nn_ptr.as_ptr(), count)
        }
    }
}

impl<T> DerefMut for HeapArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            let (nn_ptr, count) = ActiveArr::<T>::data(self.ptr);
            from_raw_parts_mut(nn_ptr.as_ptr(), count)
        }
    }
}

impl<T> HeapArray<T> {
    // pub unsafe fn from_leaked(ptr: *mut T, count: usize) -> Self {
    //     Self { ptr, count }
    // }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> (NonNull<T>, usize) {
        unsafe {
            let (nn_ptr, count) = ActiveArr::<T>::data(self.ptr);
            forget(self);
            (nn_ptr, count)
        }
    }
}

impl<T> Drop for HeapArray<T> {
    fn drop(&mut self) {
        unsafe {
            let (start, count) = ActiveArr::<T>::data(self.ptr);
            let start = start.as_ptr();
            for i in 0..count {
                drop_in_place(start.add(i));
            }
            ActiveArr::<T>::yeet(self.ptr);
        }
    }
}

// === impl HeapFixedVec ===

unsafe impl<T> Send for HeapFixedVec<T> {}

impl<T> Deref for HeapFixedVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        unsafe {
            let (nn_ptr, _count) = ActiveArr::<MaybeUninit<T>>::data(self.ptr);
            from_raw_parts(nn_ptr.as_ptr().cast::<T>(), self.len)
        }
    }
}

impl<T> DerefMut for HeapFixedVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            let (nn_ptr, _count) = ActiveArr::<MaybeUninit<T>>::data(self.ptr);
            from_raw_parts_mut(nn_ptr.as_ptr().cast::<T>(), self.len)
        }
    }
}

impl<T> HeapFixedVec<T> {
    pub fn push(&mut self, item: T) -> Result<(), T> {
        let (nn_ptr, count) = unsafe { ActiveArr::<MaybeUninit<T>>::data(self.ptr) };
        if count == self.len {
            return Err(item);
        }
        unsafe {
            nn_ptr.as_ptr().cast::<T>().add(self.len).write(item);
            self.len += 1;
        }
        Ok(())
    }
}

impl<T> Drop for HeapFixedVec<T> {
    fn drop(&mut self) {
        unsafe {
            let (start, _count) = ActiveArr::<MaybeUninit<T>>::data(self.ptr);
            let start = start.as_ptr().cast::<T>();
            for i in 0..self.len {
                drop_in_place(start.add(i));
            }
            ActiveArr::<MaybeUninit<T>>::yeet(self.ptr);
        }
    }
}
