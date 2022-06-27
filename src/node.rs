//! # `mnemos-alloc` Allocation Nodes
//!
//! These types represent the "behind the scenes" underlying types necessary
//! to safely enable the behaviors of the async allocation layer.
//!
//! These types are used by the `heap` module when allocating or freeing
//! an element, and are the "inner" types used by the `containers` module
//! to provide user-accessible types.
//!
//! This module has VERY PARTICULAR safety guarantees and concerns, and as
//! such these abstractions are not made crate-public, and kept private
//! within this module as much as is reasonably possible.

use cordyceps::{mpsc_queue::Links, Linked};
use core::mem::ManuallyDrop;
use core::{alloc::Layout, ptr::NonNull};

use crate::heap::AHeap;

/// The heap allocation Node type
///
/// The Node type is never ACTUALLY created or used directly, but instead
/// is used as a "superset" of its children to ensure that the alignment
/// and necessary size are respected at the time of allocation. Allocation
/// is ALWAYS done as a Node<T>, meaning that conversions from an active
/// type to a Recycle type are always sound.
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

/// An Active node type
///
/// This type represents a live allocation of a single item, similar to a
/// Box<T> in liballoc.
///
/// It contains a pointer to the allocator, as well as storage for the item.
///
/// The contained data MUST be valid for the lifetime of the `Active<T>`.
#[repr(C)]
pub(crate) struct Active<T> {
    heap: *const AHeap,
    data: T,
}

/// An Active Array node type
///
/// This type represents a live allocation of a dynamic number of items,
/// similar to a `Box<[T]>` in liballoc. Note that this is NOT the same as
/// a `Vec<T>`, which can be dynamically resized. The underlying storage
/// here is always a fixed size, however that fixed size is chosen at
/// runtime, rather than at compile time.
///
/// It contains a pointer to the allocator, as well as storage for the items.
///
/// The contained data MUST be valid for the lifetime of the `ActiveArr<T>`.
///
/// The ActiveArr type itself actually contains storage for zero `T` items, however
/// it uses a `[T; 0]` to force the correct alignment of the `data` field. This
/// allows us to add `size_of::<T>() * N` bytes directly following the item, which
/// can be indexed starting at the address of the `data` field. This is done by
/// over-allocating space, and using the `ActiveArr::data` function to obtain
/// access to the array storage.
///
/// NOTE: Although the `data` field is not public (even within the crate),
/// EXTREME CARE must be taken NOT to access the data field through a reference
/// to an ActiveArr type. Creating a reference (rather than a pointer) to the
/// ActiveArr type itself serves as a "narrowing" of the provenance, which means
/// that accessing out of bound elements of `data` (which is ALL of them, as
/// data "officially" has an array length of zero) is undefined behavior.
///
/// The `ActiveArr::data` function handles this by using the `addr_of!` macro
/// to obtain the pointer of the underlying array storage, WITHOUT narrowing
/// the provenance of the outer "supersized" allocation.
#[repr(C)]
pub(crate) struct ActiveArr<T> {
    heap: *const AHeap,
    capacity: usize,
    data: [T; 0],
}

/// A Recycle node type
///
/// Recycle is the "terminal state" of all allocations. After the actual
/// heap allocated data has been dropped, all active allocations become
/// a Recycle node. Allocations remain in this state until they have been
/// freed by the underlying allocator.
///
/// In the fast path, a Recycle node is dropped directly by the allocator.
/// In the slow path, the intrusive linked list header contained within
/// a Recycle node is used to "send" the allocation to a lock-free, intrusive
/// MpscQueue, where it will live until the allocator cleans up the pending
/// freelist items.
#[repr(C)]
pub(crate) struct Recycle {
    // THIS MUST be the first item!
    pub(crate) links: Links<Recycle>,
    // This is the layout of the ENTIRE Node<T>, NOT just the payload.
    pub(crate) node_layout: Layout,
}

impl<T> Active<T> {
    /// Convert an Active<T> into a Recycle, and release it to be freed
    ///
    /// This function does NOT handle dropping of the contained T, which
    /// must be done BEFORE calling this function.
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

    /// Set the heap pointer contained within the given Active<T>.
    ///
    /// This should ONLY be used to initialize the Active<T> at time of allocation.
    #[inline(always)]
    pub(crate) unsafe fn write_heap(this: NonNull<Active<T>>, heap: *const AHeap) {
        let ptr = this.as_ptr();
        core::ptr::addr_of_mut!((*ptr).heap).write(heap);
    }

    /// Obtain a pointer to the underlying data storage
    ///
    /// Although Active<T> does not have the same provenance challenges that the
    /// ActiveArr<T> type has, we use the same `data` interface for reasons of
    /// consistency. This also ensures that reordering or other modifications of
    /// the underlying node type do not require changes elsewhere.
    #[inline(always)]
    pub(crate) unsafe fn data(this: NonNull<Active<T>>) -> NonNull<T> {
        let ptr = this.as_ptr();
        let dptr = core::ptr::addr_of_mut!((*ptr).data);
        NonNull::new_unchecked(dptr)
    }
}

impl<T> ActiveArr<T> {
    /// Obtain a valid layout for an ActiveArr
    ///
    /// As we can't directly create a `Layout` type for our Node<T>/ActiveArr<T>
    /// because of the `!Sized` nature of `[T]`, we instead do manual layout
    /// surgery here instead. This function takes the alignment necessary for
    /// a `Node<T>`, but also increases the size to accomodate a `[T]` with
    /// a size of the given `ct` parameter.
    ///
    /// The given layout will always have a size >= the size of a `Node<T>`, even
    /// if the `ActiveArr<T> + [T]` would be smaller than a `Node<T>`.
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

    /// Set the heap pointer contained within the given ActiveArr<T>.
    ///
    /// This should ONLY be used to initialize the ActiveArr<T> at time of allocation.
    #[inline(always)]
    pub(crate) unsafe fn write_heap(this: NonNull<ActiveArr<T>>, heap: *const AHeap) {
        let ptr = this.as_ptr();
        core::ptr::addr_of_mut!((*ptr).heap).write(heap);
    }

    /// Obtain a pointer to the start of the array storage, as well as the length of the array
    ///
    /// NOTE: This VERY CAREFULLY avoids issues of provenance due to accessing "out of bounds"
    /// of the `data` field of the `ActiveArr` type. See the docs of the ActiveArr type for
    /// a more detailed discussion of these particularities.
    #[inline(always)]
    pub(crate) unsafe fn data(this: NonNull<ActiveArr<T>>) -> (NonNull<T>, usize) {
        let size = this.as_ref().capacity;
        let tptr = this.as_ptr();
        let daddr = core::ptr::addr_of_mut!((*tptr).data);
        let nn = NonNull::new_unchecked(daddr.cast::<T>());
        (nn, size)
    }

    /// Convert an Active<T> into a Recycle, and release it to be freed
    ///
    /// This function does NOT handle dropping of the contained `[T]`, which
    /// must be done BEFORE calling this function.
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

/// A handle that is used by the mpsc freelist to hold a linked list of Recycle nodes
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
