use core::{cell::UnsafeCell, cmp, fmt, mem::MaybeUninit};
pub use mnemos_alloc::containers::ArrayBuf;

pub struct OwnedReadBuf {
    /// The underlying owned heap buffer.
    buf: ArrayBuf<u8>,
    /// Length of the region that the user has read bytes into.
    full: usize,
    /// Length of the region of the buffer that has been initialized.
    init: usize,
}

impl OwnedReadBuf {
    /// Allocates a new, uninitialized `OwnedReadBuf`.
    ///
    /// This function allocates a buffer of the requested length, but does not
    /// initialize the allocated buffer. This function will not return until
    /// allocation succeeds.
    ///
    /// # Panics
    ///
    /// - if the provided `len` is zero.
    /// - if the provided `len` or large enough that creating the layout would
    ///   fail.
    #[must_use]
    pub async fn new(len: usize) -> Self {
        let buf = ArrayBuf::new_uninit(len).await;
        Self {
            buf,
            full: 0,
            init: 0,
        }
    }

    /// Attempts to allocate a new, uninitialized `OwnedReadBuf`.
    ///
    /// This function tries to allocate a buffer of the requested length, but
    /// does not initialize the allocated buffer. If no memory can be allocated,
    /// this function returns `None`.
    ///
    /// # Panics
    ///
    /// - if the provided `len` is zero.
    /// - if the provided `len` or large enough that creating the layout would
    ///   fail.
    #[must_use]
    pub fn try_new(len: usize) -> Option<Self> {
        let buf = ArrayBuf::try_new_uninit(len)?;
        Some(Self {
            buf,
            full: 0,
            init: 0,
        })
    }

    /// Returns the total capacity of the buffer.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Returns a shared reference to the filled portion of the buffer.
    #[inline]
    #[must_use]
    pub fn filled(&self) -> &[u8] {
        let slice = &self.buf[..self.full];
        // Safety: `self.full` describes the length of the portion of the buffer
        // that data has been read into, so we know it's initialized.
        unsafe { slice_assume_init(slice) }
    }

    /// Returns a mutable reference to the filled portion of the buffer.
    #[inline]
    #[must_use]
    pub fn filled_mut(&mut self) -> &mut [u8] {
        let slice = &mut self.buf[..self.full];
        // Safety: `self.full` describes the length of the portion of the buffer
        // that data has been read into, so we know it's initialized.
        unsafe { slice_assume_init_mut(slice) }
    }

    /// Returns a shared reference to the initialized portion of the buffer.
    ///
    /// This includes the filled portion.
    #[inline]
    #[must_use]
    pub fn initialized(&self) -> &[u8] {
        let slice = &self.buf[..self.init];
        // safety: initialized describes how far into the buffer that the user
        // has at some point initialized with bytes.
        unsafe { slice_assume_init(slice) }
    }

    /// Returns a mutable reference to the initialized portion of the buffer.
    ///
    /// This includes the filled portion.
    #[inline]
    #[must_use]
    pub fn initialized_mut(&mut self) -> &mut [u8] {
        let slice = &mut self.buf[..self.init];
        // safety: initialized describes how far into the buffer that the user
        // has at some point initialized with bytes.
        unsafe { slice_assume_init_mut(slice) }
    }

    /// Returns a mutable reference to the entire buffer, without ensuring that
    /// it has been fully initialized.
    ///
    /// The elements between 0 and `self.filled().len()` are filled, and those
    /// between 0 and `self.initialized().len()` are initialized (and so can be
    /// converted to a `&mut [u8]`).
    ///
    /// The caller of this method must ensure that these invariants are upheld.
    /// For example, if the caller initializes some of the uninitialized section
    /// of the buffer, it must call [`assume_init`](Self::assume_init) with the
    /// number of bytes initialized.
    ///
    /// # Safety
    ///
    /// The caller must not de-initialize portions of the buffer that have
    /// already been initialized. This includes any bytes in the region marked
    /// as uninitialized by `ReadBuf`.
    #[inline]
    #[must_use]
    pub unsafe fn inner_mut(&mut self) -> &mut [UnsafeCell<MaybeUninit<u8>>] {
        &mut self.buf
    }

    /// Returns a mutable reference to the unfilled part of the buffer without
    /// ensuring that it has been fully initialized.
    ///
    /// # Safety
    ///
    /// The caller must not de-initialize portions of the buffer that have
    /// already been initialized. This includes any bytes in the region marked
    /// as uninitialized by `ReadBuf`.
    #[inline]
    #[must_use]
    pub unsafe fn unfilled_mut(&mut self) -> &mut [UnsafeCell<MaybeUninit<u8>>] {
        &mut self.buf[self.full..]
    }

    /// Returns a mutable reference to the unfilled part of the buffer, ensuring
    /// it is fully initialized.
    ///
    /// Since `OwnedReadBuf` tracks the region of the buffer that has been
    /// initialized, this is effectively "free" after the first use.
    #[inline]
    #[must_use]
    pub fn zero_initialize_unfilled(&mut self) -> &mut [u8] {
        self.zero_initialize_unfilled_to(self.remaining())
    }

    /// Returns a mutable reference to the first `n` bytes of the unfilled part
    /// of the buffer, ensuring it is fully initialized.
    ///
    /// # Panics
    ///
    /// Panics if `self.remaining()` is less than `n`.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn zero_initialize_unfilled_to(&mut self, n: usize) -> &mut [u8] {
        assert!(
            self.remaining() >= n,
            "n ({n}B) overflows remaining ({}B)",
            self.remaining()
        );

        // This can't overflow, otherwise the assert above would have failed.
        let end = self.full + n;

        if self.init < end {
            unsafe {
                self.buf[self.init..end]
                    .as_mut_ptr()
                    .write_bytes(0, end - self.init);
            }
            self.init = end;
        }

        let slice = &mut self.buf[self.full..end];
        unsafe {
            // safety: we just checked that the end of the buffer has been
            // initialized as far as `n`.
            slice_assume_init_mut(slice)
        }
    }

    /// Returns the number of bytes at the end of the buffer that have not yet
    /// been filled.
    #[inline]
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.capacity() - self.full
    }

    /// Clears the buffer, resetting the filled region to empty.
    ///
    /// The number of initialized bytes is not changed, and the contents of the
    /// buffer are not modified.
    #[inline]
    pub fn clear(&mut self) {
        self.full = 0;
    }

    /// Advances the size of the filled region of the buffer.
    ///
    /// The number of initialized bytes is not changed.
    ///
    /// # Panics
    ///
    /// Panics if the filled region of the buffer would become larger than the
    /// initialized region.
    #[inline]
    #[track_caller]
    pub fn advance(&mut self, n: usize) {
        let new = self.full.checked_add(n).expect("filled overflow");
        self.set_filled(new);
    }

    /// Sets the size of the filled region of the buffer.
    ///
    /// The number of initialized bytes is not changed.
    ///
    /// Note that this can be used to *shrink* the filled region of the buffer
    /// in addition to growing it (for example, by a `AsyncRead` implementation
    /// that compresses data in-place).
    ///
    /// # Panics
    ///
    /// Panics if the filled region of the buffer would become larger than the
    /// initialized region.
    #[inline]
    #[track_caller]
    pub fn set_filled(&mut self, len: usize) {
        assert!(
            len <= self.init,
            "filled ({len}) must not become larger than initialized ({})",
            self.init
        );
        self.full = len;
    }

    /// Asserts that the first `len` unfilled bytes of the buffer are
    /// initialized.
    ///
    /// `ReadBuf` assumes that bytes are never de-initialized, so this method
    /// does nothing when called with fewer bytes than are already known to be
    /// initialized.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `len` unfilled bytes of the buffer have
    /// already been initialized.
    #[inline]
    pub unsafe fn assume_init(&mut self, len: usize) {
        let new = self.full + len;
        self.init = cmp::max(self.init, new);
    }

    /// Appends data to the buffer, advancing the written position and possibly
    /// also the initialized position.
    ///
    /// # Panics
    ///
    /// Panics if `self.remaining()` is less than `buf.len()`.
    #[inline]
    #[track_caller]
    pub fn copy_from_slice(&mut self, buf: &[u8]) {
        assert!(
            self.remaining() >= buf.len(),
            "buf.len() must fit in remaining()"
        );

        // Cannot overflow, asserted above
        let end = self.full + buf.len();
        unsafe {
            // Safety: the length is asserted above
            self.buf[self.full..end]
                .as_mut_ptr()
                .cast::<u8>()
                .copy_from_nonoverlapping(buf.as_ptr(), buf.len());
        }

        self.init = cmp::max(self.init, end);
        self.full = end;
    }
}

impl fmt::Debug for OwnedReadBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedReadBuf")
            .field("full", &self.full)
            .field("init", &self.init)
            .field("capacity", &self.capacity())
            .finish()
    }
}

unsafe fn slice_assume_init(slice: &[UnsafeCell<MaybeUninit<u8>>]) -> &[u8] {
    &*(slice as *const [UnsafeCell<MaybeUninit<u8>>] as *const [u8])
}

unsafe fn slice_assume_init_mut(slice: &mut [UnsafeCell<MaybeUninit<u8>>]) -> &mut [u8] {
    &mut *(slice as *mut [UnsafeCell<MaybeUninit<u8>>] as *mut [u8])
}
