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

    /// Release an index back to the pool.
    ///
    /// The freed index may now be returned by a subsequent call to [`allocate`](Self::allocate).
    #[inline]
    pub fn free(&self, index: u8) {
        self.0.fetch_and(!(1 << index), Release);
    }

    /// Returns `true` if all indices in the allocator have been allocated.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.load(Acquire) == u16::MAX
    }

    /// Returns `true` if none of this allocator's indices have been allocated.
    #[must_use]
    #[inline]
    pub fn is_full(&self) -> bool {
        self.0.load(Acquire) == 0
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
