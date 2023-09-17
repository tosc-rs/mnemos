use portable_atomic::{AtomicU16, Ordering::*};

/// An allocator for up to 16 unique indices.
pub struct IndexAlloc16(AtomicU16);

impl IndexAlloc16 {
    /// Returns a new allocator for up to 16 unique indices.
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU16::new(0))
    }

    /// Allocate an index from the pool.
    ///
    /// If this method returns [`Some`], the returned [`u8`] index will not be
    /// returned again until after it has been [`free`](Self::free)d.
    #[must_use]
    pub fn allocate(&self) -> Option<u8> {
        let mut bitmap = self.0.load(Acquire);
        loop {
            let idx = find_zero(bitmap)?;
            let new_bitmap = bitmap | (1 << idx);
            match self
                .0
                .compare_exchange_weak(bitmap, new_bitmap, AcqRel, Acquire)
            {
                Ok(_) => return Some(idx),
                Err(actual) => bitmap = actual,
            }
        }
    }

    /// The *total* number of indices in this allocator.
    pub const CAPACITY: u8 = 16;

    /// Release an index back to the pool.
    ///
    /// The freed index may now be returned by a subsequent call to
    /// [`allocate`](Self::allocate).
    #[inline]
    pub fn free(&self, index: u8) {
        self.0.fetch_and(!(1 << index), Release);
    }

    /// Returns `true` if *all* indices in the allocator have been allocated.
    ///
    /// This is the inverse of [`any_free`](Self::any_free).
    ///
    /// # Examples
    ///
    /// ```
    /// use mnemos_bitslab::index::IndexAlloc16;
    ///
    /// let alloc = IndexAlloc16::new();
    /// assert!(!alloc.all_allocated());
    ///
    /// // allocate all but one index
    /// for _ in 0..15 {
    ///     alloc.allocate().expect("should have free indices");
    /// assert!(!alloc.all_allocated());
    /// }
    ///
    /// // allocate the last index.
    /// let last = alloc.allocate().expect("should have one more index remaining");
    /// assert!(alloc.all_allocated());
    ///
    /// // freeing the index should make it available again
    /// alloc.free(last);
    /// assert!(!alloc.all_allocated());
    /// ```
    #[must_use]
    #[inline]
    pub fn all_allocated(&self) -> bool {
        self.0.load(Acquire) == u16::MAX
    }

    /// Returns `true` if *none* of this allocator's indices have been
    /// allocated.
    ///
    /// This is the inverse of [`any_allocated`](Self::any_allocated).
    ///
    /// # Examples
    ///
    /// ```
    /// use mnemos_bitslab::index::IndexAlloc16;
    ///
    /// let alloc = IndexAlloc16::new();
    /// assert!(alloc.all_free());
    ///
    /// let idx = alloc.allocate().expect("a fresh allocator should have indices!");
    /// assert!(!alloc.all_free());
    ///
    /// // free the last index. now, `all_free` will return `true` again.
    /// alloc.free(idx);
    /// assert!(alloc.all_free());
    /// ```
    #[must_use]
    #[inline]
    pub fn all_free(&self) -> bool {
        self.0.load(Acquire) == 0
    }

    /// Returns `true` if *any* index in the allocator has been allocated.
    ///
    /// This is the inverse of [`all_free`](Self::all_free).
    ///
    /// # Examples
    ///
    /// ```
    /// use mnemos_bitslab::index::IndexAlloc16;
    ///
    /// let alloc = IndexAlloc16::new();
    /// assert!(!alloc.any_allocated());
    ///
    /// // allocate all indices
    /// for _ in 0..16 {
    ///     alloc.allocate().expect("should have free indices");
    ///     assert!(alloc.any_allocated());
    /// }
    ///
    /// // free all but one index.
    /// for i in 0..15 {
    ///     alloc.free(i);
    ///     assert!(alloc.any_allocated());
    /// }
    ///
    /// // free the last index. now, `any_allocated` will return `false`.
    /// alloc.free(15);
    /// assert!(!alloc.any_allocated());
    /// ```
    #[must_use]
    #[inline]
    pub fn any_allocated(&self) -> bool {
        self.0.load(Acquire) != 0
    }

    /// Returns `true` if *any* index in the allocator is available.
    ///
    /// This is the inverse of [`all_allocated`](Self::all_allocated).
    ///
    /// # Examples
    ///
    /// ```
    /// use mnemos_bitslab::index::IndexAlloc16;
    ///
    /// let alloc = IndexAlloc16::new();
    /// assert!(alloc.any_free());
    ///
    /// // allocate all but one index
    /// for _ in 0..15 {
    ///     alloc.allocate().expect("should have free indices");
    ///     assert!(alloc.any_free());
    /// }
    ///
    /// // allocate the last index.
    /// let last = alloc.allocate().expect("should have one more index remaining");
    /// assert!(!alloc.any_free());
    ///
    /// // freeing the index should make it available again
    /// alloc.free(last);
    /// assert!(alloc.any_free());
    /// ```
    #[must_use]
    #[inline]
    pub fn any_free(&self) -> bool {
        self.0.load(Acquire) != u16::MAX
    }

    /// Returns the current number of free indices in the allocator.
    ///
    /// This will always be [`Self::CAPACITY`] or less.
    ///
    /// ```
    /// use mnemos_bitslab::index::IndexAlloc16;
    ///
    /// let alloc = IndexAlloc16::new();
    /// assert_eq!(alloc.free_count(), 16);
    ///
    /// let idx1 = alloc.allocate().expect("all indices should be free");
    /// assert_eq!(alloc.free_count(), 15);
    ///
    /// let idx2 = alloc.allocate().expect("15 indices should be free");
    /// assert_eq!(alloc.free_count(), 14);
    ///
    /// alloc.free(idx1);
    /// assert_eq!(alloc.free_count(), 15);
    /// ```
    #[must_use]
    #[inline]
    pub fn free_count(&self) -> u8 {
        self.0.load(Acquire).count_zeros() as u8
    }

    /// Returns the current number of allocated indices in the allocator.
    ///
    /// This will always be [`Self::CAPACITY`] or less.
    ///
    /// # Examples
    ///
    /// ```
    /// use mnemos_bitslab::index::IndexAlloc16;
    ///
    /// let alloc = IndexAlloc16::new();
    /// assert_eq!(alloc.allocated_count(), 0);
    ///
    /// let idx1 = alloc.allocate().expect("all indices should be free");
    /// assert_eq!(alloc.allocated_count(), 1);
    ///
    /// let idx2 = alloc.allocate().expect("15 indices should be free");
    /// assert_eq!(alloc.allocated_count(), 2);
    ///
    /// alloc.free(idx1);
    /// assert_eq!(alloc.allocated_count(), 1);
    /// ```
    #[must_use]
    #[inline]
    pub fn allocated_count(&self) -> u8 {
        self.0.load(Acquire).count_ones() as u8
    }
}

fn find_zero(u: u16) -> Option<u8> {
    let trailing_ones = u.trailing_ones();
    if trailing_ones == 16 {
        None
    } else {
        Some(trailing_ones as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{prop_assert_eq, proptest};

    proptest! {
        #[test]
        fn find_zero_works(u: u16) {
            let mut found_zero = None;
            for i in 0..u16::BITS {
                if u & (1 << i) == 0 {
                    found_zero = Some(i as u8);
                    break;
                }
            }

            prop_assert_eq!(find_zero(u), found_zero)
        }
    }
}
