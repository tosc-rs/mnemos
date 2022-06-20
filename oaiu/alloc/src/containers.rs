// TODO
#![allow(unused_imports, dead_code, unreachable_code)]

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

pub(crate) union Node<T> {
    active: ManuallyDrop<Active<T>>,
    recycle: ManuallyDrop<Recycle>,
}

#[repr(C)]
pub(crate) struct Active<T> {
    // THIS MUST be the first item!
    data: UnsafeCell<MaybeUninit<T>>,
    heap: NonNull<AHeap>,
}

impl<T> Active<T> {
    unsafe fn yeet(ptr: *mut Active<T>) {
        let heap = (*ptr).heap;
        core::ptr::drop_in_place((*ptr).data.get());
        let ptr: *mut Recycle = ptr.cast();

        ptr.write(Recycle {
            links: Links::new(),
            node_layout: Layout::new::<Node<T>>(),
        });

        let nn_ptr = NonNull::new_unchecked(ptr);

        (*heap.as_ptr()).release_node(nn_ptr);
    }
}

#[repr(C)]
pub(crate) struct Recycle {
    // THIS MUST be the first item!
    pub(crate) links: Links<Recycle>,
    // This is the layout of the ENTIRE Node<T>, NOT just the payload.
    pub(crate) node_layout: Layout,
}

impl<T> Drop for Node<T> {
    fn drop(&mut self) {
        panic!("Nodes should never be directly dropped!");
    }
}

pub(crate) struct NodeRef {
    pub(crate) node: NonNull<Recycle>,
}

unsafe impl Linked<Links<Recycle>> for Recycle {
    type Handle = NodeRef;

    fn into_ptr(r: Self::Handle) -> NonNull<Self> {
        r.node
    }

    unsafe fn from_ptr(ptr: NonNull<Self>) -> Self::Handle {
        NodeRef { node: ptr }
    }

    unsafe fn links(ptr: NonNull<Self>) -> NonNull<Links<Recycle>> {
        ptr.cast::<Links<Recycle>>()
    }
}


/// An Anachro Heap Box Type
pub struct HeapBox<T> {
    ptr: *mut Active<T>,
}

unsafe impl<T> Send for HeapBox<T> {}

impl<T> Deref for HeapBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr.cast::<T>() }
    }
}

impl<T> DerefMut for HeapBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.ptr.cast::<T>() }
    }
}

impl<T> HeapBox<T> {
    pub unsafe fn from_leaked(ptr: *mut T) -> Self {
        Self { ptr: ptr.cast::<Active<T>>() }
    }

    /// Leak the contents of this box, never to be recovered (probably)
    pub fn leak(self) -> &'static mut T {
        let mutref: &'static mut _ = unsafe { &mut *self.ptr.cast::<T>() };
        forget(self);
        mutref
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        unsafe {
            Active::<T>::yeet(self.ptr);
        }
    }
}
