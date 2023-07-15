//! Async-aware Container Types
//!
//! These types play well with [MnemosAlloc][crate::heap::MnemosAlloc]

use crate::heap::alloc;
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

//
// Arc
//

/// A wrapper of [`alloc::sync::Arc<T>`]
pub struct Arc<T: ?Sized> {
    inner: alloc::sync::Arc<T>,
}

// These require the same bounds as `alloc::sync::Arc`'s `Send` and `Sync`
// impls.
unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Arc<T> {
    /// Attempt to allocate a new reference counted T.
    ///
    /// Returns an error containing the provided value if the allocation
    /// could not immediately succeed.
    ///
    /// NOTE/TODO: Today this will panic if not immediately successful. This should
    /// be fixed in the future
    pub fn try_new(t: T) -> Result<Self, T> {
        Ok(Self {
            inner: alloc::sync::Arc::new(t),
        })
    }

    /// Attempt to allocate a new reference counted T.
    ///
    /// Will not complete until the allocation succeeds
    ///
    /// NOTE/TODO: Today this will panic if not immediately successful. This should
    /// be fixed in the future
    pub async fn new(t: T) -> Self {
        Self {
            inner: alloc::sync::Arc::new(t),
        }
    }

    /// Convert into a pointer
    ///
    /// This does NOT change the strong reference count
    pub fn into_raw(a: Self) -> NonNull<T> {
        unsafe { NonNull::new_unchecked(alloc::sync::Arc::into_raw(a.inner).cast_mut()) }
    }

    /// Restore from a pointer
    ///
    /// This does NOT change the strong reference count. This has the same
    /// safety invariants as [alloc::sync::Arc].
    #[inline(always)]
    pub unsafe fn from_raw(nn: NonNull<T>) -> Self {
        Self {
            inner: alloc::sync::Arc::from_raw(nn.as_ptr()),
        }
    }

    /// Increment the strong reference count
    ///
    /// This has the same afety invariants as [alloc::sync::Arc::increment_strong_count()].
    #[inline(always)]
    pub unsafe fn increment_strong_count(ptr: *const T) {
        alloc::sync::Arc::increment_strong_count(ptr)
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Deref for Arc<T> {
    type Target = alloc::sync::Arc<T>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

//
// Box
//

/// A wrapper of [`alloc::boxed::Box<T>`]
pub struct Box<T> {
    inner: alloc::boxed::Box<T>,
}

unsafe impl<T: Send> Send for Box<T> {}
unsafe impl<T: Sync> Sync for Box<T> {}

impl<T> Box<T> {
    /// Attempt to allocate a new owned T.
    ///
    /// Will not complete until the allocation succeeds.
    pub async fn new(t: T) -> Self {
        let ptr: *mut T = alloc(Layout::new::<T>()).await.cast().as_ptr();
        unsafe {
            ptr.write(t);
            Self::from_raw(ptr)
        }
    }

    /// Attempt to allocate a new owned T.
    ///
    /// Returns an error containing the provided value if the allocation
    /// could not immediately succeed.
    pub fn try_new(t: T) -> Result<Self, T> {
        match NonNull::new(unsafe { alloc::alloc::alloc(Layout::new::<T>()) }) {
            Some(ptr) => unsafe {
                let ptr = ptr.cast::<T>().as_ptr();
                ptr.write(t);
                Ok(Self {
                    inner: alloc::boxed::Box::from_raw(ptr),
                })
            },
            None => Err(t),
        }
    }

    /// Convert into a pointer
    pub fn into_raw(me: Self) -> *mut T {
        alloc::boxed::Box::into_raw(me.inner)
    }

    /// Convert from a pointer
    ///
    /// This has the same safety invariants as [alloc::boxed::Box::from_raw()]
    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        Self {
            inner: alloc::boxed::Box::from_raw(ptr),
        }
    }

    /// Convert to a regular old alloc box
    pub fn into_alloc_box(self) -> alloc::boxed::Box<T> {
        self.inner
    }
}

impl<T> Deref for Box<T> {
    type Target = alloc::boxed::Box<T>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for Box<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

//
// ArrayBuf
//

/// A spooky owned array type
///
/// This type represents ownership of essentially an `UnsafeCell<MaybeUninit<[T]>>`.
///
/// It is intended as a low level building block for things like bbqueue and other data
/// structures that need to own a specific number of items, and would like to set their
/// own safety invariants, without manually using `alloc`.
pub struct ArrayBuf<T> {
    ptr: NonNull<UnsafeCell<MaybeUninit<T>>>,
    len: usize,
}

unsafe impl<T: Send> Send for ArrayBuf<T> {}
unsafe impl<T: Sync> Sync for ArrayBuf<T> {}

impl<T> ArrayBuf<T> {
    /// Gets the layout for `len` items
    ///
    /// Panics if creating the layout would fail (e.g. too large for the platform)
    fn layout(len: usize) -> Layout {
        Layout::array::<UnsafeCell<MaybeUninit<T>>>(len).unwrap()
    }

    /// Try to allocate a new ArrayBuf with storage for `len` items.
    ///
    /// Returns None if the allocation does not succeed immediately.
    ///
    /// Panics if the len is zero, or large enough that creating the layout would fail
    pub fn try_new_uninit(len: usize) -> Option<Self> {
        assert_ne!(len, 0, "ZST ArrayBuf doesn't make sense");
        let layout = Self::layout(len);
        let ptr = NonNull::new(unsafe { alloc::alloc::alloc(layout) })?.cast();
        Some(ArrayBuf { ptr, len })
    }

    /// Try to allocate a new ArrayBuf with storage for `len` items.
    ///
    /// Will not return until allocation succeeds.
    ///
    /// Panics if the len is zero, or large enough that creating the layout would fail
    pub async fn new_uninit(len: usize) -> Self {
        assert_ne!(len, 0, "ZST ArrayBuf doesn't make sense");
        let layout = Self::layout(len);
        let ptr = alloc(layout).await.cast();
        ArrayBuf { ptr, len }
    }

    /// Obtain a pointer to the heap allocated storage, as well as the length of items
    ///
    /// This does NOT leak the heap allocation. The returned pointer has the lifetime
    /// of this `ArrayBuf`.
    pub fn ptrlen(&self) -> (NonNull<UnsafeCell<MaybeUninit<T>>>, usize) {
        (self.ptr, self.len)
    }

    /// Returns the length of the `ArrayBuf`.
    #[inline]
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.len
    }
}

impl<T> Drop for ArrayBuf<T> {
    fn drop(&mut self) {
        debug_assert_ne!(self.len, 0, "how did you do that");
        let layout = Self::layout(self.len);
        unsafe {
            alloc::alloc::dealloc(self.ptr.as_ptr().cast(), layout);
        }
    }
}

impl<T> Deref for ArrayBuf<T> {
    type Target = [UnsafeCell<MaybeUninit<T>>];
    fn deref(&self) -> &Self::Target {
        unsafe {
            // Safety: the `ArrayBuf` logically owns `self.ptr`, and it is only
            // deallocated when the `ArrayBuf` is dropped. The `ArrayBuf` was
            // allocated with a layout of `self.len` `T`s, and thus the
            // constructed slice should not exceed the bounds of the allocation.
            core::slice::from_raw_parts(self.ptr.as_ptr(), self.len)
        }
    }
}

impl<T> DerefMut for ArrayBuf<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            // Safety: the `ArrayBuf` logically owns `self.ptr`, and it is only
            // deallocated when the `ArrayBuf` is dropped. The `ArrayBuf` was
            // allocated with a layout of `self.len` `T`s, and thus the
            // constructed slice should not exceed the bounds of the allocation.
            core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len)
        }
    }
}

//
// ArrayBuf
//

/// A spooky owned array type
///
/// This type represents ownership of essentially an `UnsafeCell<MaybeUninit<[T]>>`.
///
/// It is intended as a low level building block for things like bbqueue and other data
/// structures that need to own a specific number of items, and would like to set their
/// own safety invariants, without manually using `alloc`.
pub struct HeapArray<T> {
    ptr: NonNull<T>,
    len: usize,
}

unsafe impl<T: Send> Send for HeapArray<T> {}
unsafe impl<T: Sync> Sync for HeapArray<T> {}

impl<T> HeapArray<T> {
    /// Gets the layout for `len` items
    ///
    /// Panics if creating the layout would fail (e.g. too large for the platform)
    fn layout(len: usize) -> Layout {
        Layout::array::<T>(len).unwrap()
    }

    /// Try to allocate a new HeapArray with storage for `len` items.
    ///
    /// Will not return until allocation succeeds.
    ///
    /// Panics if the len is zero, or large enough that creating the layout would fail
    pub async fn new(len: usize, init: T) -> Self
    where
        T: Copy,
    {
        assert_ne!(len, 0, "ZST HeapArray doesn't make sense");
        let layout = Self::layout(len);
        let ptr: NonNull<T> = alloc(layout).await.cast();
        unsafe {
            let ptr = ptr.as_ptr();
            for i in 0..len {
                ptr.add(i).write(init);
            }
        }
        HeapArray { ptr, len }
    }

    // /// Obtain a pointer to the heap allocated storage, as well as the length of items
    // ///
    // /// This does NOT leak the heap allocation. The returned pointer has the lifetime
    // /// of this `HeapArray`.
    // pub fn ptrlen(&self) -> (NonNull<UnsafeCell<MaybeUninit<T>>>, usize) {
    //     (self.ptr, self.len)
    // }

    /// Returns the length of the `HeapArray`.
    #[inline]
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.len
    }
}

impl<T> Drop for HeapArray<T> {
    fn drop(&mut self) {
        debug_assert_ne!(self.len, 0, "how did you do that");
        let layout = Self::layout(self.len);
        unsafe {
            alloc::alloc::dealloc(self.ptr.as_ptr().cast(), layout);
        }
    }
}

impl<T> Deref for HeapArray<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe {
            // Safety: the `HeapArray` logically owns `self.ptr`, and it is only
            // deallocated when the `HeapArray` is dropped. The `HeapArray` was
            // allocated with a layout of `self.len` `T`s, and thus the
            // constructed slice should not exceed the bounds of the allocation.
            core::slice::from_raw_parts(self.ptr.as_ptr(), self.len)
        }
    }
}

impl<T> DerefMut for HeapArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            // Safety: the `ArrayBuf` logically owns `self.ptr`, and it is only
            // deallocated when the `ArrayBuf` is dropped. The `ArrayBuf` was
            // allocated with a layout of `self.len` `T`s, and thus the
            // constructed slice should not exceed the bounds of the allocation.
            core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len)
        }
    }
}

//
// FixedVec
//

/// A `Vec` with a fixed upper size
///
/// Semantically, [FixedVec] works basically the same as [alloc::vec::Vec], however
/// [FixedVec] will NOT ever reallocate to increase size. In practice, this acts like
/// a heap allocated version of heapless' Vec type.
pub struct FixedVec<T> {
    inner: alloc::vec::Vec<T>,
}

unsafe impl<T: Send> Send for FixedVec<T> {}
unsafe impl<T: Sync> Sync for FixedVec<T> {}

impl<T> FixedVec<T> {
    /// Try to allocate a new FixedVec with storage for UP TO `capacity` items.
    ///
    /// Returns None if the allocation does not succeed immediately.
    ///
    /// Panics if the len is zero, or large enough that creating the layout would fail
    pub fn try_new(capacity: usize) -> Option<Self> {
        assert_ne!(capacity, 0, "ZST FixedVec doesn't make sense");
        let layout = Layout::array::<T>(capacity).unwrap();

        unsafe {
            let ptr = NonNull::new(alloc::alloc::alloc(layout))?;
            return Some(FixedVec {
                inner: alloc::vec::Vec::from_raw_parts(ptr.cast().as_ptr(), 0, capacity),
            });
        }
    }

    /// Try to allocate a new FixedVec with storage for UP TO `capacity` items.
    ///
    /// Will not return until allocation succeeds.
    ///
    /// Panics if the len is zero, or large enough that creating the layout would fail
    pub async fn new(capacity: usize) -> Self {
        assert_ne!(capacity, 0, "ZST FixedVec doesn't make sense");
        let layout = Layout::array::<T>(capacity).unwrap();

        unsafe {
            let ptr = alloc(layout).await;
            return FixedVec {
                inner: alloc::vec::Vec::from_raw_parts(ptr.cast().as_ptr(), 0, capacity),
            };
        }
    }

    /// Attempt to push an item into the fixed vec.
    ///
    /// Returns an error if the fixed vec is full
    #[inline]
    pub fn try_push(&mut self, t: T) -> Result<(), T> {
        if self.is_full() {
            Err(t)
        } else {
            self.inner.push(t);
            Ok(())
        }
    }

    /// Removes the last element from a vector and returns it, or [`None`] if it
    /// is empty.
    ///
    /// This method is identical to the [`Vec::pop`](alloc::vec::Vec::pop)
    /// method in `liballoc`.
    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop()
    }

    /// Attempt to push an item into the fixed vec.
    ///
    /// Returns an error if the slice would not fit in the capacity.
    /// If an error is returned, the contents of the FixedVec is unchanged
    #[inline]
    pub fn try_extend_from_slice(&mut self, sli: &[T]) -> Result<(), ()>
    where
        T: Clone,
    {
        let new_len = match self.inner.len().checked_add(sli.len()) {
            Some(c) => c,
            None => return Err(()),
        };

        if new_len > self.inner.capacity() {
            return Err(());
        }

        self.inner.extend_from_slice(sli);
        Ok(())
    }

    /// Obtain a reference to the underlying [alloc::vec::Vec]
    #[inline]
    pub fn as_vec(&self) -> &alloc::vec::Vec<T> {
        &self.inner
    }

    /// Get inner mutable vec
    ///
    /// SAFETY:
    ///
    /// You must not do anything that could realloc or increase the capacity.
    /// We want an exact upper limit.
    ///
    /// This would not be memory unsafe, but would violate the invariants of [FixedVec],
    /// which is supposed to have a fixed upper size.
    #[inline]
    pub unsafe fn as_vec_mut(&mut self) -> &mut alloc::vec::Vec<T> {
        &mut self.inner
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// In other words, remove all elements `e` for which `f(&e)` returns `false`.
    /// This method operates in place, visiting each element exactly once in the
    /// original order, and preserves the order of the retained elements.
    ///
    /// This method is identical to the
    /// [`Vec::retain`](alloc::vec::Vec::retain) method in `liballoc`.
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.inner.retain(f)
    }

    /// Retains only the elements specified by the predicate, passing a mutable reference to it.
    ///
    /// In other words, remove all elements `e` such that `f(&mut e)` returns `false`.
    /// This method operates in place, visiting each element exactly once in the
    /// original order, and preserves the order of the retained elements.
    ///
    /// This method is identical to the
    /// [`Vec::retain_mut`](alloc::vec::Vec::retain_mut) method in `liballoc`.
    pub fn retain_mut<F>(&mut self, f: F)
    where
        F: FnMut(&mut T) -> bool,
    {
        self.inner.retain_mut(f)
    }

    /// Obtain a reference to the current contents
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    /// Obtain a mutable reference to the current contents
    #[inline]
    pub fn as_slice_mut(&mut self) -> &mut [T] {
        &mut self.inner
    }

    /// Clear the FixedVec
    ///
    /// This method is identical to the [`Vec::clear`](alloc::vec::Vec::clear)
    /// method in `liballoc`.
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Is the FixedVec full?
    #[inline]
    pub fn is_full(&self) -> bool {
        self.inner.len() == self.inner.capacity()
    }

    /// Returns `true` if this `FixedVec` is empty (its [`len`](Self::len) is
    /// 0).
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the length of the `FixedVec`.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns the total capacity in this `FixedVec`.
    ///
    /// This method is identical to the
    /// [`Vec::capacity`](alloc::vec::Vec::capacity) method in `liballoc`.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}

impl<T> AsRef<[T]> for FixedVec<T> {
    #[inline(always)]
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> AsMut<[T]> for FixedVec<T> {
    #[inline(always)]
    fn as_mut(&mut self) -> &mut [T] {
        self.as_slice_mut()
    }
}
