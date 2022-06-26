// TODO
#![allow(unused_imports, dead_code, unreachable_code)]

use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    mem::{align_of, forget, size_of},
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering, AtomicBool, AtomicUsize}, pin::Pin,
};
use heapless::mpmc::MpMcQueue;
use linked_list_allocator::Heap;
use maitake::wait::WaitQueue;
use cordyceps::{mpsc_queue::{MpscQueue, Links}, Linked};

use crate::heap::AHeap;
use crate::node::{Active, ActiveArr};

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
        unsafe {
            let act = self.ptr.as_ref();
            &*act.data.get().cast::<T>()
        }
    }
}

impl<T> DerefMut for HeapBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            let act = self.ptr.as_mut();
            &mut *act.data.get().cast::<T>()
        }
    }
}

impl<T> HeapBox<T> {
    // pub unsafe fn from_leaked(ptr: *mut T) -> Self {
    //     Self { ptr: ptr.cast::<Active<T>>() }
    // }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> &'static mut T {
        let mutref: &'static mut _ = unsafe { &mut *(*self.ptr.as_ptr()).data.get().cast::<T>() };
        forget(self);
        mutref
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        unsafe {
            let item_ptr = self.ptr.as_mut().data.get().cast::<T>();
            core::ptr::drop_in_place(item_ptr);
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
            let count = self.ptr.as_ref().capacity;
            let ptr = self.ptr.as_ref().data.get().cast::<T>();
            core::slice::from_raw_parts(ptr, count)
        }
    }
}

impl<T> DerefMut for HeapArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            let count = self.ptr.as_mut().capacity;
            let ptr = self.ptr.as_mut().data.get().cast::<T>();
            core::slice::from_raw_parts_mut(ptr, count)
        }
    }
}

impl<T> HeapArray<T> {
    // pub unsafe fn from_leaked(ptr: *mut T, count: usize) -> Self {
    //     Self { ptr, count }
    // }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(mut self) -> &'static mut [T] {
        unsafe {
            let count = self.ptr.as_mut().capacity;
            let ptr = self.ptr.as_mut().data.get().cast::<T>();
            let mutref = unsafe { core::slice::from_raw_parts_mut(ptr, count) };
            forget(self);
            mutref
        }
    }
}

// impl<T> Drop for HeapArray<T> {
//     fn drop(&mut self) {

//         unsafe {
//             core::ptr::drop_in_place(self.ptr);
//             Active::<T>::yeet(self.ptr);
//         }

//         todo!()
//     }
// }
