use core::mem::ManuallyDrop;
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
};
use cordyceps::{mpsc_queue::Links, Linked};

use crate::heap::AHeap;

// So, I never ACTUALLY make a `Node`, but rather use it as a sort
// of superset alignment + size type when allocating. I ONLY allocate
// Node<T>, which means access to each of them is acceptable
#[allow(dead_code)]
#[repr(C)]
pub(crate) union Node<T> {
    // These are "active" types - e.g. they contain a live allocation
    active: ManuallyDrop<Active<T>>,
    active_arr: ManuallyDrop<ActiveArr<T>>,

    // This is the "recycle" type - after the contents of the allocation
    // has been retired, but the node still needs to be dropped via the
    // actual underlying allocator
    recycle: ManuallyDrop<Recycle>,
}

#[repr(C)]
pub(crate) struct Active<T> {
    heap: *const AHeap,
    data: UnsafeCell<MaybeUninit<T>>,
}

#[repr(C)]
pub(crate) struct ActiveArr<T> {
    heap: *const AHeap,
    capacity: usize,
    data: UnsafeCell<MaybeUninit<[T; 0]>>,
}

#[repr(C)]
pub(crate) struct Recycle {
    // THIS MUST be the first item!
    pub(crate) links: Links<Recycle>,
    // This is the layout of the ENTIRE Node<T>, NOT just the payload.
    pub(crate) node_layout: Layout,
}

impl<T> Active<T> {
    /// Convert an Active<T> into a Recycle, and free it
    #[inline]
    pub(crate) unsafe fn yeet(mut ptr: NonNull<Active<T>>) {
        let heap = ptr.as_mut().heap;
        let ptr: NonNull<Recycle> = ptr.cast();

        ptr.as_ptr().write(Recycle {
            links: Links::new(),
            node_layout: Layout::new::<Node<T>>(),
        });

        (*heap).release_node(ptr);
    }

    #[inline(always)]
    pub(crate) unsafe fn write_heap(this: NonNull<Active<T>>, heap: *const AHeap) {
        let ptr = this.as_ptr();
        core::ptr::addr_of_mut!((*ptr).heap).write(heap);
    }

    #[inline(always)]
    pub(crate) unsafe fn data(this: NonNull<Active<T>>) -> NonNull<T> {
        let dptr = this.as_ref().data.get().cast::<T>();
        NonNull::new_unchecked(dptr)
    }
}

impl<T> ActiveArr<T> {
    #[inline]
    pub(crate) unsafe fn layout_for_arr(ct: usize) -> Layout {
        let layout_node = Layout::new::<Node<T>>();
        let layout_acta = Layout::new::<ActiveArr<T>>();
        let arr_size = core::mem::size_of::<T>() * ct;
        let size = layout_acta.size() + arr_size;
        let size = core::cmp::max(layout_node.size(), size);

        // We take the ALIGNMENT from the `Node`, which is a superset
        // type, and the SIZE from either the (ActiveArr + Array) OR
        // Node, whichever is larger
        Layout::from_size_align_unchecked(size, layout_node.align())
    }

    #[inline(always)]
    pub(crate) unsafe fn write_heap(this: NonNull<ActiveArr<T>>, heap: *const AHeap) {
        let ptr = this.as_ptr();
        core::ptr::addr_of_mut!((*ptr).heap).write(heap);
    }

    #[inline(always)]
    pub(crate) unsafe fn data(this: NonNull<ActiveArr<T>>) -> (NonNull<T>, usize) {
        let size = this.as_ref().capacity;
        let tptr = this.as_ptr();
        let daddr = core::ptr::addr_of_mut!((*tptr).data);
        let nn = NonNull::new_unchecked(daddr.cast::<T>());
        (nn, size)
    }

    /// Convert an Active<T> into a Recycle, and free it
    #[inline]
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
