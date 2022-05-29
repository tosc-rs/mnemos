use core::{sync::atomic::{AtomicUsize, Ordering}, cell::UnsafeCell, marker::PhantomData, ops::{Deref, DerefMut}, ptr::NonNull};


/// Atomic ReFCell - ArfCell
///
/// Like a refcell (or RwLock), but atomic, has a const constructor,
/// and doesn't panic
pub struct ArfCell<T> {
    state: AtomicUsize,
    item: UnsafeCell<T>,
}

pub struct MutArfGuard<'a, T> {
    cell: NonNull<ArfCell<T>>,
    plt: PhantomData<&'a mut T>,
}

impl<'a, T> Deref for MutArfGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            &*self.cell.as_ref().item.get()
        }
    }
}

impl<'a, T> DerefMut for MutArfGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            self.cell.as_mut().item.get_mut()
        }
    }
}

impl<'a, T> Drop for MutArfGuard<'a, T> {
    fn drop(&mut self) {
        unsafe {
            self.cell.as_ref().state.store(0, Ordering::Release);
        }
    }
}

pub struct ArfGuard<'a, T> {
    cell: NonNull<ArfCell<T>>,
    plt: PhantomData<&'a mut T>,
}

impl<'a, T> Deref for ArfGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            &*self.cell.as_ref().item.get()
        }
    }
}

impl<'a, T> Drop for ArfGuard<'a, T> {
    fn drop(&mut self) {
        unsafe {
            let x = self.cell.as_ref().state.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(x != 0, "Underflow on refcnt release!");
        }
    }
}

impl<T> ArfCell<T> {
    const MUTLOCK: usize = (usize::MAX / 2) + 1;

    pub const fn new(item: T) -> Self {
        ArfCell {
            state: AtomicUsize::new(0),
            item: UnsafeCell::new(item),
        }
    }

    pub fn borrow_mut<'a>(&'a self) -> Result<MutArfGuard<'a, T>, ()> {
        self.state.compare_exchange(
            0,
            Self::MUTLOCK,
            // TODO: Relax these
            Ordering::SeqCst,
            Ordering::SeqCst
        ).map_err(drop)?;

        Ok(MutArfGuard {
            cell: unsafe { NonNull::new_unchecked(self as *const Self as *mut Self) },
            plt: PhantomData,
        })
    }

    pub fn borrow<'a>(&'a self) -> Result<ArfGuard<'a, T>, ()> {
        // proactive check we aren't mutably locked
        if self.state.load(Ordering::Acquire) >= Self::MUTLOCK {
            return Err(());
        }

        // TODO: Check the old value to see if we're close to overflowing the refcnt?

        // Now fetch-add, and see how it goes
        let old = self.state.fetch_add(1, Ordering::AcqRel);
        if old >= Self::MUTLOCK {
            // Oops, we raced with a mutable lock. We lose.
            // It's okay we incremented `state` here anyway - the mutable lock will
            // unconditionally reset to zero, and future borrowers will hopefully be
            // caught by the proactive check above to reduce the chance of
            // overflowing the refcount.
            return Err(());
        }

        Ok(ArfGuard {
            cell: unsafe { NonNull::new_unchecked(self as *const Self as *mut Self) },
            plt: PhantomData,
        })
    }
}
