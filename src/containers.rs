use crate::node::{Active, ActiveArr};
use core::marker::PhantomData;
use core::ptr::drop_in_place;
use core::slice::{from_raw_parts, from_raw_parts_mut};
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

/// An Anachro Heap Array Type
pub struct HeapArray<T> {
    pub(crate) ptr: NonNull<ActiveArr<T>>,
    pub(crate) pd: PhantomData<Active<T>>,
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
    // pub unsafe fn from_leaked(ptr: *mut T) -> Self {
    //     Self { ptr: ptr.cast::<Active<T>>() }
    // }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> &'static mut T {
        let mutref: &'static mut _ = unsafe { &mut *Active::<T>::data(self.ptr).as_ptr() };
        forget(self);
        mutref
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
    pub fn leak(self) -> &'static mut [T] {
        unsafe {
            let (nn_ptr, count) = ActiveArr::<T>::data(self.ptr);
            let mutref = from_raw_parts_mut(nn_ptr.as_ptr(), count);
            forget(self);
            mutref
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
