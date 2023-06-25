use crate::heap::alloc;
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

// Arc

pub struct Arc<T: ?Sized> {
    inner: alloc::sync::Arc<T>,
}

// These require the same bounds as `alloc::sync::Arc`'s `Send` and `Sync`
// impls.
unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Arc<T> {
    pub fn try_new(t: T) -> Result<Self, T> {
        // TODO: Failable way of allocating arcs?
        Ok(Self {
            inner: alloc::sync::Arc::new(t),
        })
    }

    pub async fn new(t: T) -> Self {
        // TODO: Async way of allocating Arc?
        Self {
            inner: alloc::sync::Arc::new(t),
        }
    }

    pub fn into_raw(a: Self) -> NonNull<T> {
        unsafe { NonNull::new_unchecked(alloc::sync::Arc::into_raw(a.inner).cast_mut()) }
    }

    #[inline(always)]
    pub unsafe fn from_raw(nn: NonNull<T>) -> Self {
        Self {
            inner: alloc::sync::Arc::from_raw(nn.as_ptr()),
        }
    }

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

// Box

pub struct Box<T> {
    inner: alloc::boxed::Box<T>,
}

unsafe impl<T: Send> Send for Box<T> {}
unsafe impl<T: Sync> Sync for Box<T> {}

impl<T> Box<T> {
    pub fn into_raw(me: Self) -> *mut T {
        alloc::boxed::Box::into_raw(me.inner)
    }

    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        Self {
            inner: alloc::boxed::Box::from_raw(ptr),
        }
    }

    pub async fn new(t: T) -> Self {
        let ptr: *mut T = alloc(Layout::new::<T>()).await.cast().as_ptr();
        unsafe {
            ptr.write(t);
            Self::from_raw(ptr)
        }
    }

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

pub struct ArrayBuf<T> {
    ptr: NonNull<UnsafeCell<MaybeUninit<T>>>,
    _pd: PhantomData<*const T>,
    len: usize,
}

unsafe impl<T: Send> Send for ArrayBuf<T> {}
unsafe impl<T: Sync> Sync for ArrayBuf<T> {}

impl<T> ArrayBuf<T> {
    fn layout(len: usize) -> Layout {
        Layout::array::<UnsafeCell<MaybeUninit<T>>>(len).unwrap()
    }

    pub fn try_new_uninit(len: usize) -> Option<Self> {
        assert_ne!(len, 0, "ZST ArrayBuf doesn't make sense");
        let layout = Self::layout(len);
        let ptr = NonNull::new(unsafe { alloc::alloc::alloc(layout) })?.cast();
        Some(ArrayBuf {
            ptr,
            _pd: PhantomData,
            len,
        })
    }

    pub async fn new_uninit(len: usize) -> Self {
        assert_ne!(len, 0, "ZST ArrayBuf doesn't make sense");
        let layout = Self::layout(len);
        let ptr = alloc(layout).await.cast();
        ArrayBuf {
            ptr,
            _pd: PhantomData,
            len,
        }
    }

    pub fn ptrlen(&self) -> (NonNull<UnsafeCell<MaybeUninit<T>>>, usize) {
        (self.ptr, self.len)
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

// FixedVec

pub struct FixedVec<T> {
    inner: alloc::vec::Vec<T>,
}

unsafe impl<T: Send> Send for FixedVec<T> {}
unsafe impl<T: Sync> Sync for FixedVec<T> {}

impl<T> FixedVec<T> {
    pub fn try_new(capacity: usize) -> Option<Self> {
        let layout = Layout::array::<T>(capacity).unwrap();

        unsafe {
            let ptr = NonNull::new(alloc::alloc::alloc(layout))?;
            return Some(FixedVec {
                inner: alloc::vec::Vec::from_raw_parts(ptr.cast().as_ptr(), 0, capacity),
            });
        }
    }

    pub async fn new(capacity: usize) -> Self {
        let layout = Layout::array::<T>(capacity).unwrap();

        unsafe {
            let ptr = alloc(layout).await;
            return FixedVec {
                inner: alloc::vec::Vec::from_raw_parts(ptr.cast().as_ptr(), 0, capacity),
            };
        }
    }

    #[inline]
    pub fn try_push(&mut self, t: T) -> Result<(), T> {
        if self.is_full() {
            Err(t)
        } else {
            self.inner.push(t);
            Ok(())
        }
    }

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

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    #[inline]
    pub fn as_slice_mut(&mut self) -> &mut [T] {
        &mut self.inner
    }

    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.inner.len() == self.inner.capacity()
    }
}
