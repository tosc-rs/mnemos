
use core::mem::ManuallyDrop;
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
};
use cordyceps::{mpsc_queue::Links, Linked};

use crate::heap::AHeap;

pub(crate) union Node<T> {
    active: ManuallyDrop<Active<T>>,
    active_arr: ManuallyDrop<ActiveArr<T>>,
    recycle: ManuallyDrop<Recycle>,
}

#[repr(C)]
pub(crate) struct Active<T> {
    pub(crate) heap: *const AHeap,
    pub(crate) data: UnsafeCell<MaybeUninit<T>>,
}

#[repr(C)]
pub(crate) struct ActiveArr<T> {
    pub(crate) heap: *const AHeap,
    pub(crate) capacity: usize,
    pub(crate) data: UnsafeCell<MaybeUninit<[T; 0]>>,
}

impl<T> Active<T> {
    pub(crate) unsafe fn yeet(mut ptr: NonNull<Active<T>>) {
        let heap = ptr.as_mut().heap;
        let ptr: NonNull<Recycle> = ptr.cast();

        ptr.as_ptr().write(Recycle {
            links: Links::new(),
            node_layout: Layout::new::<Node<T>>(),
        });

        (*heap).release_node(ptr);
    }
}

impl<T> ActiveArr<T> {
    pub(crate) unsafe fn layout_for_arr(ct: usize) -> Layout {
        let layout_node = Layout::new::<Node<T>>();
        let layout_acta = Layout::new::<ActiveArr<T>>();
        let arr_size = core::mem::size_of::<T>() * ct;
        let size = layout_acta.size() + arr_size;
        let size = core::cmp::max(layout_node.size(), size);

        Layout::from_size_align_unchecked(size, layout_node.align())
    }

    pub(crate) unsafe fn yeet(mut ptr: NonNull<ActiveArr<T>>) {
        let heap = ptr.as_mut().heap;
        let capacity = ptr.as_mut().capacity;

        let ptr: NonNull<Recycle> = ptr.cast();
        let layout = Self::layout_for_arr(capacity);

        ptr.as_ptr().write(Recycle {
            links: Links::new(),
            node_layout: layout,
        });

        (*heap).release_node(ptr);
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
