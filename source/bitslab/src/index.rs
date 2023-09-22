macro_rules! make_index_allocs {
    (
        $(
            mod $modname:ident {
                pub struct $Name:ident($Atomic:ty, $Int:ty, $capacity:literal);
            }
        )+
    ) => {
        $(
            pub use self::$modname::$Name;
            mod $modname {
                use portable_atomic::{$Atomic, Ordering::*};

                #[doc = concat!("An allocator for up to ", stringify!($cap), " unique indices.")]
                pub struct $Name($Atomic);

                impl $Name {
                    #[doc = concat!("An allocator for up to ", stringify!($cap), " unique indices.")]
                    #[must_use]
                    pub const fn new() -> Self {
                        Self(<$Atomic>::new(0))
                    }

                    /// Allocate an index from the pool.
                    ///
                    /// If this method returns [`Some`], the returned [`u8`] index will not be
                    /// returned again until after it has been [`free`](Self::free)d.
                    #[must_use]
                    pub fn allocate(&self) -> Option<u8> {
                        let mut bitmap = self.0.load(Acquire);
                        loop {
                            let idx = Self::find_zero(bitmap)?;
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
                    const CAPACITY: u8 = $capacity;

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
                    #[doc = concat!(" use mnemos_bitslab::index::", stringify!($Name), ";")]
                    ///
                    #[doc = concat!(" let alloc = ", stringify!($Name), "::new();")]
                    /// assert!(!alloc.all_allocated());
                    ///
                    /// // allocate all but one index
                    #[doc = concat!(" for _ in 1..", stringify!($capacity), " {")]
                    ///     alloc.allocate().expect("should have free indices");
                    ///     assert!(!alloc.all_allocated());
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
                        self.0.load(Acquire) == <$Int>::MAX
                    }

                    /// Returns `true` if *none* of this allocator's indices have been
                    /// allocated.
                    ///
                    /// This is the inverse of [`any_allocated`](Self::any_allocated).
                    ///
                    /// # Examples
                    ///
                    /// ```
                    #[doc = concat!(" use mnemos_bitslab::index::", stringify!($Name), ";")]
                    ///
                    #[doc = concat!(" let alloc = ", stringify!($Name), "::new();")]
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
                    #[doc = concat!(" use mnemos_bitslab::index::", stringify!($Name), ";")]
                    ///
                    #[doc = concat!(" let alloc = ", stringify!($Name), "::new();")]
                    /// assert!(!alloc.any_allocated());
                    ///
                    /// // allocate all indices
                    #[doc = concat!(" for _ in 0..", stringify!($capacity), " {")]
                    ///     alloc.allocate().expect("should have free indices");
                    ///     assert!(alloc.any_allocated());
                    /// }
                    ///
                    /// // free all but one index.
                    #[doc = concat!(" for i in 1..", stringify!($capacity), " {")]
                    ///     alloc.free(i);
                    ///     assert!(alloc.any_allocated());
                    /// }
                    ///
                    /// // free the last index. now, `any_allocated` will return `false`.
                    /// alloc.free(0);
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
                    #[doc = concat!(" use mnemos_bitslab::index::", stringify!($Name), ";")]
                    ///
                    #[doc = concat!(" let alloc = ", stringify!($Name), "::new();")]
                    /// assert!(alloc.any_free());
                    ///
                    /// // allocate all but one index
                    #[doc = concat!(" for _ in 1..", stringify!($capacity), " {")]
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
                        self.0.load(Acquire) != <$Int>::MAX
                    }

                    /// Returns the current number of free indices in the allocator.
                    ///
                    /// This will always be [`self.capacity()`] or less.
                    ///
                    /// ```
                    #[doc = concat!(" use mnemos_bitslab::index::", stringify!($Name), ";")]
                    ///
                    #[doc = concat!(" let alloc = ", stringify!($Name), "::new();")]
                    /// assert_eq!(alloc.free_count(), alloc.capacity());
                    ///
                    /// let idx1 = alloc.allocate().expect("all indices should be free");
                    /// assert_eq!(alloc.free_count(), alloc.capacity() - 1);
                    ///
                    /// let idx2 = alloc.allocate().expect("most indices should be free");
                    /// assert_eq!(alloc.free_count(), alloc.capacity() - 2);
                    ///
                    /// alloc.free(idx1);
                    /// assert_eq!(alloc.free_count(), alloc.capacity() - 1);
                    /// ```
                    #[must_use]
                    #[inline]
                    pub fn free_count(&self) -> u8 {
                        self.0.load(Acquire).count_zeros() as u8
                    }

                    /// Returns the current number of allocated indices in the allocator.
                    ///
                    /// This will always be [`self.capacity()`] or less.
                    ///
                    /// # Examples
                    ///
                    /// ```
                    #[doc = concat!(" use mnemos_bitslab::index::", stringify!($Name), ";")]
                    ///
                    #[doc = concat!(" let alloc = ", stringify!($Name), "::new();")]
                    /// assert_eq!(alloc.allocated_count(), 0);
                    ///
                    /// let idx1 = alloc.allocate().expect("all indices should be free");
                    /// assert_eq!(alloc.allocated_count(), 1);
                    ///
                    /// let idx2 = alloc.allocate().expect("most indices should be free");
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

                    /// Returns the total capacity of this allocator, including any
                    /// allocated indices.
                    #[must_use]
                    #[inline]
                    pub const fn capacity(&self) -> u8 {
                        Self::CAPACITY
                    }

                    fn find_zero(u: $Int) -> Option<u8> {
                        let trailing_ones = u.trailing_ones();
                        if trailing_ones == $capacity {
                            None
                        } else {
                            Some(trailing_ones as u8)
                        }
                    }
                }

                #[cfg(test)]
                mod tests {
                    use super::*;
                    use proptest::{prop_assert_eq, proptest};

                    proptest! {
                        #[test]
                        fn find_zero_works(u: $Int) {
                            let mut found_zero = None;
                            for i in 0..<$Int>::BITS as $Int {
                                if u & (1 << i) == 0 {
                                    found_zero = Some(i as u8);
                                    break;
                                }
                            }

                            prop_assert_eq!($Name::find_zero(u), found_zero)
                        }
                    }
                }
            }
        )+
    };
}

make_index_allocs! {
    mod alloc8 {
        pub struct IndexAlloc8(AtomicU8, u8, 8);
    }

    mod alloc16 {
        pub struct IndexAlloc16(AtomicU16, u16, 16);
    }

    mod alloc32 {
        pub struct IndexAlloc32(AtomicU32, u32, 32);
    }

    mod alloc64 {
        pub struct IndexAlloc64(AtomicU64, u64, 64);
    }
}
