/// An iterator over a *snapshot* of the currently allocated indices in an index
/// allocator.
#[derive(Debug, Clone)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct AllocatedIndices {
    map: u64,
    idx: u8,
    end: u8,
}

macro_rules! make_index_allocs {
    (
        $(
            mod $modname:ident {
                pub struct $Name:ident($Atomic:ty, $Int:ty, $capacity:expr);
            }
        )+
    ) => {
        $(
            pub use self::$modname::$Name;
            mod $modname {
                use portable_atomic::{$Atomic, Ordering::*};
                use core::fmt;

                #[doc = concat!("An allocator for up to ", stringify!($cap), " unique indices.")]
                pub struct $Name {
                    bitmap: $Atomic,
                    max_mask: $Int,
                }

                impl $Name {
                    #[doc = concat!("Returns a new allocator for up to ", stringify!($cap), " unique indices.")]
                    #[must_use]
                    pub const fn new() -> Self {
                        Self {
                            bitmap: <$Atomic>::new(0),
                            max_mask: 0,
                        }
                    }

                    /// Returns a new allocator for up to `capacity` unique
                    /// indices. If `capacity` indices are allocated, subsequent
                    /// calls to [`allocate()`](Self::allocate) will return
                    /// [`None`] until an index is deallocated by a call to
                    /// [`free()`](Self::free) on this allocator.
                    ///
                    #[doc = concat!("A `", stringify!($Name), "` can only ever allocate up to [`Self::MAX_CAPACITY`] indices.")]
                    /// Therefore, if the provided `capacity` exceeds
                    /// [`Self::MAX_CAPACITY`], it will be clamped to the
                    /// maximum capacity.
                    ///
                    /// An allocator's actual capacity can be returned
                    pub const fn with_capacity(capacity: u8) -> Self {
                        let capacity = if capacity > Self::MAX_CAPACITY {
                            Self::MAX_CAPACITY
                        } else {
                            capacity
                        };

                        // if capacity is less than max capacity, mask out the
                        // highest (MAX_CAPACITY - capacity) bits;
                        let mut max_mask: $Int = 0;
                        let mut i = Self::MAX_CAPACITY;
                        while i > capacity {
                            i -= 1;
                            max_mask |= 1 << i;
                        }

                        Self {
                            bitmap: <$Atomic>::new(max_mask),
                            max_mask,
                        }
                    }

                    /// Allocate an index from the pool.
                    ///
                    /// If this method returns [`Some`], the returned [`u8`] index will not be
                    /// returned again until after it has been [`free`](Self::free)d.
                    #[must_use]
                    pub fn allocate(&self) -> Option<u8> {
                        let mut bitmap = self.bitmap.load(Acquire);
                        loop {
                            let idx = Self::find_zero(bitmap)?;
                            let new_bitmap = bitmap | (1 << idx);
                            match self
                                .bitmap
                                .compare_exchange_weak(bitmap, new_bitmap, AcqRel, Acquire)
                            {
                                Ok(_) => return Some(idx),
                                Err(actual) => bitmap = actual,
                            }
                        }
                    }

                    /// The maximum number of indices that can be allocated by
                    /// an allocator of this type.
                    pub const MAX_CAPACITY: u8 = $capacity as u8;

                    /// Release an index back to the pool.
                    ///
                    /// The freed index may now be returned by a subsequent call to
                    /// [`allocate`](Self::allocate).
                    #[inline]
                    pub fn free(&self, index: u8) {
                        debug_assert!(index < self.capacity());
                        self.bitmap.fetch_and(!(1 << index), Release);
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
                        self.bitmap.load(Acquire) == <$Int>::MAX
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
                        self.bitmap.load(Acquire) & !self.max_mask == 0
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
                    #[doc = concat!(" for i in 1..", stringify!($Name), "::MAX_CAPACITY {")]
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
                        self.bitmap.load(Acquire) & !self.max_mask != 0
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
                        self.bitmap.load(Acquire) != <$Int>::MAX
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
                        self.bitmap.load(Acquire).count_zeros() as u8
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
                        (self.bitmap.load(Acquire) & !self.max_mask).count_ones() as u8
                    }

                    /// Returns the total capacity of this allocator, including any
                    /// allocated indices.
                    #[must_use]
                    #[inline]
                    pub const fn capacity(&self) -> u8 {
                        Self::MAX_CAPACITY - self.capacity_subtractor()
                    }

                    /// Returns an iterator over the indices that have been
                    /// allocated *at the current point in time*.
                    #[inline]
                    #[must_use]
                    pub fn iter_allocated(&self) -> super::AllocatedIndices {
                        let map = self.bitmap.load(Acquire) & !self.max_mask;
                        let end = self.capacity();
                        super::AllocatedIndices {
                            map: map as u64, end, idx: 0,
                        }
                    }

                    #[inline]
                    const fn capacity_subtractor(&self) -> u8 {
                        self.max_mask.leading_ones() as u8
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

                impl fmt::Debug for $Name {
                    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        let Self { max_mask, bitmap } = self;
                        let bitmap = bitmap.load(Acquire);
                        f.debug_struct(stringify!($Name))
                            .field("bitmap", &format_args!("{bitmap:0width$b}", width = Self::MAX_CAPACITY as usize))
                            .field("max_mask", &format_args!("{max_mask:0width$b}", width = Self::MAX_CAPACITY as usize))
                            .finish()

                    }
                }

                #[cfg(test)]
                mod tests {
                    use super::*;
                    use std::collections::BTreeSet;
                    use proptest::prelude::*;

                    prop_compose! {
                        fn cap_with_frees()
                            (n in 1..=<$Int>::BITS as u8)
                            (n in Just(n), frees in proptest::collection::btree_set(0..n, 0..n as usize))
                            -> (u8, BTreeSet<u8>)
                        {
                            (n, frees)
                        }
                    }

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

                        #[test]
                        fn max_capacity(capacity in 1..=<$Int>::BITS as u8) {
                            let alloc = $Name::with_capacity(capacity);
                            eprintln!("capacity: {capacity};\nalloc: {alloc:#?}");
                            prop_assert_eq!(alloc.capacity(), capacity, "capacity ({}) should equal requested capacity ({})", alloc.capacity(), capacity);
                            for i in 0..capacity {
                                eprintln!("{i}");
                                prop_assert_eq!(alloc.any_allocated(), i > 0, "if i > 0, `any_allocated` must be true");
                                prop_assert!(alloc.any_free(), "if we haven't allocated the whole capacity, `any_free` must be true; i = {}", i);
                                prop_assert_eq!(alloc.all_free(), i == 0);
                                let allocated = alloc.allocate();
                                eprintln!("allocated = {allocated:?}");
                                prop_assert_eq!(allocated, Some(i));

                                prop_assert_eq!(
                                    alloc.free_count(),
                                    capacity - (i + 1),
                                    "`free_count` must be capacity ({}) - (i + 1) ({}) = {}",
                                    capacity, i + 1,
                                    capacity - (i + 1),
                                );
                                prop_assert_eq!(alloc.allocated_count(), i + 1, "we just allocated the i-th index (i = {})", i);
                                prop_assert!(alloc.any_allocated());

                                prop_assert_eq!(alloc.any_free(), i < capacity - 1, "if we haven't allocated the whole capacity, `any_free` must be true; i = {}", i);
                                prop_assert_eq!(alloc.all_allocated(), i == capacity - 1);
                            }

                            prop_assert_eq!(alloc.allocate(), None);
                            prop_assert_eq!(alloc.free_count(), 0, "all indices should be allocated so free count should be 0");
                            prop_assert_eq!(alloc.allocated_count(), capacity);
                            prop_assert!(alloc.all_allocated());
                            prop_assert!(alloc.any_allocated());
                            prop_assert!(!alloc.all_free());

                            alloc.free(capacity - 1);
                            prop_assert_eq!(alloc.allocate(), Some(capacity - 1));
                        }

                        #[test]
                        fn iter(n in 1..=<$Int>::BITS as u8) {
                            let alloc = $Name::new();
                            for i in 0..n {
                                let idx = alloc.allocate();
                                prop_assert_eq!(idx, Some(i));
                            }

                            let mut iter = alloc.iter_allocated();
                            let mut cnt = 0;

                            prop_assert_eq!(iter.size_hint(), (n as usize, Some(n as usize)));
                            while let Some(idx) = iter.next() {
                                prop_assert!(cnt <= n);
                                prop_assert_eq!(idx, cnt);

                                cnt += 1;

                                let rem = (n - cnt) as usize;
                                prop_assert_eq!(iter.size_hint(), (rem, Some(rem as usize)));
                            }
                        }

                        #[test]
                        fn iter_with_frees((n, frees) in cap_with_frees()) {
                            let alloc = $Name::new();
                            let mut idxs = BTreeSet::new();
                            for i in 0..n {
                                let idx = alloc.allocate();
                                prop_assert_eq!(idx, Some(i));
                                idxs.insert(idx.unwrap());
                            }

                            for idx in frees {
                                alloc.free(idx);
                                prop_assert!(idxs.remove(&idx));
                            }

                            let iter = alloc.iter_allocated();

                            prop_assert_eq!(iter.size_hint(), (idxs.len(), Some(idxs.len())));
                            let expected = idxs.into_iter().collect::<Vec<_>>();
                            let actual = iter.collect::<Vec<_>>();
                            prop_assert_eq!(actual, expected);
                        }
                    }
                }
            }
        )+
    };
}

impl Iterator for AllocatedIndices {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        while self.idx < self.end {
            let idx = self.idx;
            self.idx += 1;
            if self.map & (1 << idx) != 0 {
                return Some(idx);
            }
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // don't overflow when shifting to construct the mask.
        if self.idx == 64 {
            return (0, Some(0));
        }

        let mask: u64 = !((1 << self.idx) - 1);
        let rem = (self.map & mask).count_ones() as usize;
        (rem, Some(rem))
    }
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

    mod allocword {
        pub struct IndexAllocWord(AtomicUsize, usize, usize::BITS);
    }
}
