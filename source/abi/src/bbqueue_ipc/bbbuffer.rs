use core::{
    cmp::min,
    marker::PhantomData,
    mem::{forget, transmute},
    ops::{Deref, DerefMut},
    ptr::NonNull,
    slice::from_raw_parts_mut,
    sync::atomic::{
        compiler_fence, AtomicBool, AtomicPtr, AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed, Release, SeqCst},
    },
};

use crate::bbqueue_ipc::{
    framed::{FrameConsumer, FrameProducer},
    Error, Result,
};
#[derive(Debug)]
#[repr(C)]
/// A backing structure for a BBQueue. Can be used to create either
/// a BBQueue or a split Producer/Consumer pair
pub struct BBBuffer {
    buf: AtomicPtr<u8>,

    buf_len: AtomicUsize,

    /// Where the next byte will be written
    write: AtomicUsize,

    /// Where the next byte will be read from
    read: AtomicUsize,

    /// Used in the inverted case to mark the end of the
    /// readable streak. Otherwise will == sizeof::<self.buf>().
    /// Writer is responsible for placing this at the correct
    /// place when entering an inverted condition, and Reader
    /// is responsible for moving it back to sizeof::<self.buf>()
    /// when exiting the inverted condition
    last: AtomicUsize,

    /// Used by the Writer to remember what bytes are currently
    /// allowed to be written to, but are not yet ready to be
    /// read from
    reserve: AtomicUsize,

    /// Is there an active read grant?
    read_in_progress: AtomicBool,

    /// Is there an active write grant?
    write_in_progress: AtomicBool,
}

unsafe impl Sync for BBBuffer {}

impl<'a> BBBuffer {
    pub unsafe fn initialize(&'a self, buf_start: *mut u8, buf_len: usize) {
        // Explicitly zero the data to avoid undefined behavior.
        // This is required, because we hand out references to the buffers,
        // which mean that creating them as references is technically UB for now
        compiler_fence(SeqCst);
        buf_start.write_bytes(0u8, buf_len);
        self.buf_len.store(buf_len, SeqCst);
        self.buf.store(buf_start, SeqCst);
    }

    #[inline]
    pub unsafe fn take_producer(me: *mut Self) -> Producer<'static> {
        let nn_me = NonNull::new_unchecked(me);
        Producer {
            bbq: nn_me,
            pd: PhantomData,
        }
    }

    #[inline]
    pub unsafe fn take_consumer(me: *mut Self) -> Consumer<'static> {
        let nn_me = NonNull::new_unchecked(me);
        Consumer {
            bbq: nn_me,
            pd: PhantomData,
        }
    }

    #[inline]
    pub unsafe fn take_framed_producer(me: *mut Self) -> FrameProducer<'static> {
        let nn_me = NonNull::new_unchecked(me);
        FrameProducer {
            producer: Producer {
                bbq: nn_me,
                pd: PhantomData,
            },
        }
    }

    #[inline]
    pub unsafe fn take_framed_consumer(me: *mut Self) -> FrameConsumer<'static> {
        let nn_me = NonNull::new_unchecked(me);
        FrameConsumer {
            consumer: Consumer {
                bbq: nn_me,
                pd: PhantomData,
            },
        }
    }
}

impl Default for BBBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl BBBuffer {
    /// Create a new constant inner portion of a `BBBuffer`.
    ///
    /// NOTE: This is only necessary to use when creating a `BBBuffer` at static
    /// scope, and is generally never used directly. This process is necessary to
    /// work around current limitations in `const fn`, and will be replaced in
    /// the future.
    ///
    pub const fn new() -> Self {
        Self {
            // This will not be initialized until we split the buffer
            buf: AtomicPtr::new(core::ptr::null_mut()),

            buf_len: AtomicUsize::new(0),

            // Owned by the writer
            write: AtomicUsize::new(0),

            // Owned by the reader
            read: AtomicUsize::new(0),

            // Cooperatively owned
            //
            // NOTE: This should generally be initialized as size_of::<self.buf>(), however
            // this would prevent the structure from being entirely zero-initialized,
            // and can cause the .data section to be much larger than necessary. By
            // forcing the `last` pointer to be zero initially, we place the structure
            // in an "inverted" condition, which will be resolved on the first commited
            // bytes that are written to the structure.
            //
            // When read == last == write, no bytes will be allowed to be read (good), but
            // write grants can be given out (also good).
            last: AtomicUsize::new(0),

            // Owned by the Writer, "private"
            reserve: AtomicUsize::new(0),

            // Owned by the Reader, "private"
            read_in_progress: AtomicBool::new(false),

            // Owned by the Writer, "private"
            write_in_progress: AtomicBool::new(false),
        }
    }
}

/// `Producer` is the primary interface for pushing data into a `BBBuffer`.
/// There are various methods for obtaining a grant to write to the buffer, with
/// different potential tradeoffs. As all grants are required to be a contiguous
/// range of data, different strategies are sometimes useful when making the decision
/// between maximizing usage of the buffer, and ensuring a given grant is successful.
///
/// As a short summary of currently possible grants:
///
/// * `grant_exact(N)`
///   * User will receive a grant `sz == N` (or receive an error)
///   * This may cause a wraparound if a grant of size N is not available
///       at the end of the ring.
///   * If this grant caused a wraparound, the bytes that were "skipped" at the
///       end of the ring will not be available until the reader reaches them,
///       regardless of whether the grant commited any data or not.
///   * Maximum possible waste due to skipping: `N - 1` bytes
/// * `grant_max_remaining(N)`
///   * User will receive a grant `0 < sz <= N` (or receive an error)
///   * This will only cause a wrap to the beginning of the ring if exactly
///       zero bytes are available at the end of the ring.
///   * Maximum possible waste due to skipping: 0 bytes
///
/// See [this github issue](https://github.com/jamesmunns/bbqueue/issues/38) for a
/// discussion of grant methods that could be added in the future.
pub struct Producer<'a> {
    bbq: NonNull<BBBuffer>,
    pd: PhantomData<&'a ()>,
}

unsafe impl<'a> Send for Producer<'a> {}
unsafe impl<'a> Sync for Producer<'a> {}

impl<'a> Producer<'a> {
    /// Request a writable, contiguous section of memory of exactly
    /// `sz` bytes. If the buffer size requested is not available,
    /// an error will be returned.
    ///
    /// This method may cause the buffer to wrap around early if the
    /// requested space is not available at the end of the buffer, but
    /// is available at the beginning
    pub fn grant_exact(&self, sz: usize) -> Result<GrantW<'a>> {
        let inner = unsafe { &self.bbq.as_ref() };

        if atomic::swap(&inner.write_in_progress, true, AcqRel) {
            return Err(Error::GrantInProgress);
        }

        // Writer component. Must never write to `read`,
        // be careful writing to `load`
        let write = inner.write.load(Acquire);
        let read = inner.read.load(Acquire);
        let max = inner.buf_len.load(Relaxed);
        let already_inverted = write < read;

        let start = if already_inverted {
            if (write + sz) < read {
                // Inverted, room is still available
                write
            } else {
                // Inverted, no room is available
                inner.write_in_progress.store(false, Release);
                return Err(Error::InsufficientSize);
            }
        } else if write + sz <= max {
            // Non inverted condition
            write
        } else {
            // Not inverted, but need to go inverted

            // NOTE: We check sz < read, NOT <=, because
            // write must never == read in an inverted condition, since
            // we will then not be able to tell if we are inverted or not
            if sz < read {
                // Invertible situation
                0
            } else {
                // Not invertible, no space
                inner.write_in_progress.store(false, Release);
                return Err(Error::InsufficientSize);
            }
        };
        // Safe write, only viewed by this task
        inner.reserve.store(start + sz, Release);

        // This is sound, as UnsafeCell, MaybeUninit, and GenericArray
        // are all `#[repr(Transparent)]
        let start_of_buf_ptr = inner.buf.load(Relaxed);
        let grant_slice = unsafe { from_raw_parts_mut(start_of_buf_ptr.add(start), sz) };

        Ok(GrantW {
            buf: grant_slice,
            bbq: self.bbq,
            to_commit: 0,
        })
    }

    /// Request a writable, contiguous section of memory of up to
    /// `sz` bytes. If a buffer of size `sz` is not available without
    /// wrapping, but some space (0 < available < sz) is available without
    /// wrapping, then a grant will be given for the remaining size at the
    /// end of the buffer. If no space is available for writing, an error
    /// will be returned.
    // TODO(AJM): Not used in mnemos
    #[allow(dead_code)]
    pub fn grant_max_remaining(&self, mut sz: usize) -> Result<GrantW<'a>> {
        let inner = unsafe { &self.bbq.as_ref() };

        if atomic::swap(&inner.write_in_progress, true, AcqRel) {
            return Err(Error::GrantInProgress);
        }

        // Writer component. Must never write to `read`,
        // be careful writing to `load`
        let write = inner.write.load(Acquire);
        let read = inner.read.load(Acquire);
        let max = inner.buf_len.load(Relaxed);

        let already_inverted = write < read;

        let start = if already_inverted {
            // In inverted case, read is always > write
            let remain = read - write - 1;

            if remain != 0 {
                sz = min(remain, sz);
                write
            } else {
                // Inverted, no room is available
                inner.write_in_progress.store(false, Release);
                return Err(Error::InsufficientSize);
            }
        } else if write != max {
            // Some (or all) room remaining in un-inverted case
            sz = min(max - write, sz);
            write
        } else {
            // Not inverted, but need to go inverted

            // NOTE: We check read > 1, NOT read >= 1, because
            // write must never == read in an inverted condition, since
            // we will then not be able to tell if we are inverted or not
            if read > 1 {
                sz = min(read - 1, sz);
                0
            } else {
                // Not invertible, no space
                inner.write_in_progress.store(false, Release);
                return Err(Error::InsufficientSize);
            }
        };

        // Safe write, only viewed by this task
        inner.reserve.store(start + sz, Release);

        // This is sound, as UnsafeCell, MaybeUninit, and GenericArray
        // are all `#[repr(Transparent)]
        let start_of_buf_ptr = inner.buf.load(Relaxed);
        let grant_slice = unsafe { from_raw_parts_mut(start_of_buf_ptr.add(start), sz) };

        Ok(GrantW {
            buf: grant_slice,
            bbq: self.bbq,
            to_commit: 0,
        })
    }
}

/// `Consumer` is the primary interface for reading data from a `BBBuffer`.
pub struct Consumer<'a> {
    bbq: NonNull<BBBuffer>,
    pd: PhantomData<&'a ()>,
}

unsafe impl<'a> Send for Consumer<'a> {}
unsafe impl<'a> Sync for Consumer<'a> {}

impl<'a> Consumer<'a> {
    /// Obtains a contiguous slice of committed bytes. This slice may not
    /// contain ALL available bytes, if the writer has wrapped around. The
    /// remaining bytes will be available after all readable bytes are
    /// released
    pub fn read(&self) -> Result<GrantR<'a>> {
        let inner = unsafe { &self.bbq.as_ref() };

        if atomic::swap(&inner.read_in_progress, true, AcqRel) {
            return Err(Error::GrantInProgress);
        }

        let write = inner.write.load(Acquire);
        let last = inner.last.load(Acquire);
        let mut read = inner.read.load(Acquire);

        // Resolve the inverted case or end of read
        if (read == last) && (write < read) {
            read = 0;
            // This has some room for error, the other thread reads this
            // Impact to Grant:
            //   Grant checks if read < write to see if inverted. If not inverted, but
            //     no space left, Grant will initiate an inversion, but will not trigger it
            // Impact to Commit:
            //   Commit does not check read, but if Grant has started an inversion,
            //   grant could move Last to the prior write position
            // MOVING READ BACKWARDS!
            inner.read.store(0, Release);
        }

        let sz = if write < read {
            // Inverted, only believe last
            last
        } else {
            // Not inverted, only believe write
            write
        } - read;

        if sz == 0 {
            inner.read_in_progress.store(false, Release);
            return Err(Error::InsufficientSize);
        }

        // This is sound, as UnsafeCell, MaybeUninit, and GenericArray
        // are all `#[repr(Transparent)]
        let start_of_buf_ptr = inner.buf.load(Relaxed);
        let grant_slice = unsafe { from_raw_parts_mut(start_of_buf_ptr.add(read), sz) };

        Ok(GrantR {
            buf: grant_slice,
            bbq: self.bbq,
            to_release: 0,
        })
    }

    /// Obtains two disjoint slices, which are each contiguous of committed bytes.
    /// Combined these contain all previously commited data.
    #[allow(dead_code)] // TODO(AJM): Not used in mnemos
    pub fn split_read(&self) -> Result<SplitGrantR<'a>> {
        let inner = unsafe { &self.bbq.as_ref() };

        if atomic::swap(&inner.read_in_progress, true, AcqRel) {
            return Err(Error::GrantInProgress);
        }

        let write = inner.write.load(Acquire);
        let last = inner.last.load(Acquire);
        let mut read = inner.read.load(Acquire);

        // Resolve the inverted case or end of read
        if (read == last) && (write < read) {
            read = 0;
            // This has some room for error, the other thread reads this
            // Impact to Grant:
            //   Grant checks if read < write to see if inverted. If not inverted, but
            //     no space left, Grant will initiate an inversion, but will not trigger it
            // Impact to Commit:
            //   Commit does not check read, but if Grant has started an inversion,
            //   grant could move Last to the prior write position
            // MOVING READ BACKWARDS!
            inner.read.store(0, Release);
        }

        let (sz1, sz2) = if write < read {
            // Inverted, only believe last
            (last - read, write)
        } else {
            // Not inverted, only believe write
            (write - read, 0)
        };

        if sz1 == 0 {
            inner.read_in_progress.store(false, Release);
            return Err(Error::InsufficientSize);
        }

        let start_of_buf_ptr = inner.buf.load(Relaxed);
        let grant_slice1 = unsafe { from_raw_parts_mut(start_of_buf_ptr.add(read), sz1) };
        let grant_slice2 = unsafe { from_raw_parts_mut(start_of_buf_ptr, sz2) };

        Ok(SplitGrantR {
            buf1: grant_slice1,
            buf2: grant_slice2,
            bbq: self.bbq,
            to_release: 0,
        })
    }
}

/// A structure representing a contiguous region of memory that
/// may be written to, and potentially "committed" to the queue.
///
/// NOTE: If the grant is dropped without explicitly commiting
/// the contents, or by setting a the number of bytes to
/// automatically be committed with `to_commit()`, then no bytes
/// will be comitted for writing.
///
/// if the target doesn't have atomics, dropping the grant
/// without committing it takes a short critical section,
#[derive(Debug, PartialEq)]
pub struct GrantW<'a> {
    pub(crate) buf: &'a mut [u8],
    bbq: NonNull<BBBuffer>,
    pub(crate) to_commit: usize,
}

unsafe impl<'a> Send for GrantW<'a> {}

/// A structure representing a contiguous region of memory that
/// may be read from, and potentially "released" (or cleared)
/// from the queue
///
/// NOTE: If the grant is dropped without explicitly releasing
/// the contents, or by setting the number of bytes to automatically
/// be released with `to_release()`, then no bytes will be released
/// as read.
///
///
/// if the target doesn't have atomics, dropping the grant
/// without releasing it takes a short critical section,
#[derive(Debug, PartialEq)]
pub struct GrantR<'a> {
    pub(crate) buf: &'a mut [u8],
    bbq: NonNull<BBBuffer>,
    pub(crate) to_release: usize,
}

/// A structure representing up to two contiguous regions of memory that
/// may be read from, and potentially "released" (or cleared)
/// from the queue
#[derive(Debug, PartialEq)]
pub struct SplitGrantR<'a> {
    pub(crate) buf1: &'a mut [u8],
    pub(crate) buf2: &'a mut [u8],
    bbq: NonNull<BBBuffer>,
    pub(crate) to_release: usize,
}

unsafe impl<'a> Send for GrantR<'a> {}

unsafe impl<'a> Send for SplitGrantR<'a> {}

impl<'a> GrantW<'a> {
    /// Finalizes a writable grant given by `grant()` or `grant_max()`.
    /// This makes the data available to be read via `read()`. This consumes
    /// the grant.
    ///
    /// If `used` is larger than the given grant, the maximum amount will
    /// be commited
    ///
    /// NOTE: if the target doesn't have atomics, this function takes a short critical
    /// section while committing.
    pub fn commit(mut self, used: usize) {
        self.commit_inner(used);
        forget(self);
    }

    /// Obtain access to the inner buffer for writing
    pub fn buf(&mut self) -> &mut [u8] {
        self.buf
    }

    /// Sometimes, it's not possible for the lifetimes to check out. For example,
    /// if you need to hand this buffer to a function that expects to receive a
    /// `&'static mut [u8]`, it is not possible for the inner reference to outlive the
    /// grant itself.
    ///
    /// You MUST guarantee that in no cases, the reference that is returned here outlives
    /// the grant itself. Once the grant has been released, referencing the data contained
    /// WILL cause undefined behavior.
    ///
    /// Additionally, you must ensure that a separate reference to this data is not created
    /// to this data, e.g. using `DerefMut` or the `buf()` method of this grant.
    pub unsafe fn as_static_mut_buf(&mut self) -> &'static mut [u8] {
        transmute::<&mut [u8], &'static mut [u8]>(self.buf)
    }

    #[inline(always)]
    pub(crate) fn commit_inner(&mut self, used: usize) {
        let inner = unsafe { &self.bbq.as_ref() };

        // If there is no grant in progress, return early. This
        // generally means we are dropping the grant within a
        // wrapper structure
        if !inner.write_in_progress.load(Acquire) {
            return;
        }

        // Writer component. Must never write to READ,
        // be careful writing to LAST

        // Saturate the grant commit
        let len = self.buf.len();
        let used = min(len, used);

        let write = inner.write.load(Acquire);
        atomic::fetch_sub(&inner.reserve, len - used, AcqRel);

        let max = inner.buf_len.load(Relaxed);
        let last = inner.last.load(Acquire);
        let new_write = inner.reserve.load(Acquire);

        if (new_write < write) && (write != max) {
            // We have already wrapped, but we are skipping some bytes at the end of the ring.
            // Mark `last` where the write pointer used to be to hold the line here
            inner.last.store(write, Release);
        } else if new_write > last {
            // We're about to pass the last pointer, which was previously the artificial
            // end of the ring. Now that we've passed it, we can "unlock" the section
            // that was previously skipped.
            //
            // Since new_write is strictly larger than last, it is safe to move this as
            // the other thread will still be halted by the (about to be updated) write
            // value
            inner.last.store(max, Release);
        }
        // else: If new_write == last, either:
        // * last == max, so no need to write, OR
        // * If we write in the end chunk again, we'll update last to max next time
        // * If we write to the start chunk in a wrap, we'll update last when we
        //     move write backwards

        // Write must be updated AFTER last, otherwise read could think it was
        // time to invert early!
        inner.write.store(new_write, Release);

        // Allow subsequent grants
        inner.write_in_progress.store(false, Release);
    }

    /// Configures the amount of bytes to be commited on drop.
    pub fn to_commit(&mut self, amt: usize) {
        self.to_commit = self.buf.len().min(amt);
    }
}

impl<'a> GrantR<'a> {
    /// Release a sequence of bytes from the buffer, allowing the space
    /// to be used by later writes. This consumes the grant.
    ///
    /// If `used` is larger than the given grant, the full grant will
    /// be released.
    ///
    /// NOTE: if the target doesn't have atomics, this function takes a short critical
    /// section while releasing.
    pub fn release(mut self, used: usize) {
        // Saturate the grant release
        let used = min(self.buf.len(), used);

        self.release_inner(used);
        forget(self);
    }

    pub(crate) fn shrink(&mut self, len: usize) {
        let mut new_buf: &mut [u8] = &mut [];
        core::mem::swap(&mut self.buf, &mut new_buf);
        let (new, _) = new_buf.split_at_mut(len);
        self.buf = new;
    }

    /// Obtain access to the inner buffer for reading
    pub fn buf(&self) -> &[u8] {
        self.buf
    }

    /// Obtain mutable access to the read grant
    ///
    /// This is useful if you are performing in-place operations
    /// on an incoming packet, such as decryption
    pub fn buf_mut(&mut self) -> &mut [u8] {
        self.buf
    }

    /// Sometimes, it's not possible for the lifetimes to check out. For example,
    /// if you need to hand this buffer to a function that expects to receive a
    /// `&'static [u8]`, it is not possible for the inner reference to outlive the
    /// grant itself.
    ///
    /// You MUST guarantee that in no cases, the reference that is returned here outlives
    /// the grant itself. Once the grant has been released, referencing the data contained
    /// WILL cause undefined behavior.
    ///
    /// Additionally, you must ensure that a separate reference to this data is not created
    /// to this data, e.g. using `Deref` or the `buf()` method of this grant.
    pub unsafe fn as_static_buf(&self) -> &'static [u8] {
        transmute::<&[u8], &'static [u8]>(self.buf)
    }

    #[inline(always)]
    pub(crate) fn release_inner(&mut self, used: usize) {
        let inner = unsafe { &self.bbq.as_ref() };

        // If there is no grant in progress, return early. This
        // generally means we are dropping the grant within a
        // wrapper structure
        if !inner.read_in_progress.load(Acquire) {
            return;
        }

        // This should always be checked by the public interfaces
        debug_assert!(used <= self.buf.len());

        // This should be fine, purely incrementing
        let _ = atomic::fetch_add(&inner.read, used, Release);

        inner.read_in_progress.store(false, Release);
    }

    /// Configures the amount of bytes to be released on drop.
    pub fn to_release(&mut self, amt: usize) {
        self.to_release = self.buf.len().min(amt);
    }
}

impl<'a> SplitGrantR<'a> {
    /// Release a sequence of bytes from the buffer, allowing the space
    /// to be used by later writes. This consumes the grant.
    ///
    /// If `used` is larger than the given grant, the full grant will
    /// be released.
    ///
    /// NOTE: if the target doesn't have atomics, this function takes a short critical
    /// section while releasing.
    pub fn release(mut self, used: usize) {
        // Saturate the grant release
        let used = min(self.combined_len(), used);

        self.release_inner(used);
        forget(self);
    }

    /// Obtain access to both inner buffers for reading
    pub fn bufs(&self) -> (&[u8], &[u8]) {
        (self.buf1, self.buf2)
    }

    /// Obtain mutable access to both parts of the read grant
    ///
    /// This is useful if you are performing in-place operations
    /// on an incoming packet, such as decryption
    pub fn bufs_mut(&mut self) -> (&mut [u8], &mut [u8]) {
        (self.buf1, self.buf2)
    }

    #[inline(always)]
    pub(crate) fn release_inner(&mut self, used: usize) {
        let inner = unsafe { &self.bbq.as_ref() };

        // If there is no grant in progress, return early. This
        // generally means we are dropping the grant within a
        // wrapper structure
        if !inner.read_in_progress.load(Acquire) {
            return;
        }

        // This should always be checked by the public interfaces
        debug_assert!(used <= self.combined_len());

        if used <= self.buf1.len() {
            // This should be fine, purely incrementing
            let _ = atomic::fetch_add(&inner.read, used, Release);
        } else {
            // Also release parts of the second buffer
            inner.read.store(used - self.buf1.len(), Release);
        }

        inner.read_in_progress.store(false, Release);
    }

    /// Configures the amount of bytes to be released on drop.
    pub fn to_release(&mut self, amt: usize) {
        self.to_release = self.combined_len().min(amt);
    }

    /// The combined length of both buffers
    pub fn combined_len(&self) -> usize {
        self.buf1.len() + self.buf2.len()
    }
}

impl<'a> Drop for GrantW<'a> {
    fn drop(&mut self) {
        self.commit_inner(self.to_commit)
    }
}

impl<'a> Drop for GrantR<'a> {
    fn drop(&mut self) {
        self.release_inner(self.to_release)
    }
}

impl<'a> Drop for SplitGrantR<'a> {
    fn drop(&mut self) {
        self.release_inner(self.to_release)
    }
}

impl<'a> Deref for GrantW<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.buf
    }
}

impl<'a> DerefMut for GrantW<'a> {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.buf
    }
}

impl<'a> Deref for GrantR<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.buf
    }
}

impl<'a> DerefMut for GrantR<'a> {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.buf
    }
}

#[cfg(not(target_has_atomic))]
mod atomic {
    use core::sync::atomic::{
        AtomicBool, AtomicUsize,
        Ordering::{self, Acquire, Release},
    };

    use cortex_m::interrupt::free;

    #[inline(always)]
    pub fn fetch_add(atomic: &AtomicUsize, val: usize, _order: Ordering) -> usize {
        free(|_| {
            let prev = atomic.load(Acquire);
            atomic.store(prev.wrapping_add(val), Release);
            prev
        })
    }

    #[inline(always)]
    pub fn fetch_sub(atomic: &AtomicUsize, val: usize, _order: Ordering) -> usize {
        free(|_| {
            let prev = atomic.load(Acquire);
            atomic.store(prev.wrapping_sub(val), Release);
            prev
        })
    }

    #[inline(always)]
    pub fn swap(atomic: &AtomicBool, val: bool, _order: Ordering) -> bool {
        free(|_| {
            let prev = atomic.load(Acquire);
            atomic.store(val, Release);
            prev
        })
    }
}

#[cfg(target_has_atomic)]
mod atomic {
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[inline(always)]
    pub fn fetch_add(atomic: &AtomicUsize, val: usize, order: Ordering) -> usize {
        atomic.fetch_add(val, order)
    }

    #[inline(always)]
    pub fn fetch_sub(atomic: &AtomicUsize, val: usize, order: Ordering) -> usize {
        atomic.fetch_sub(val, order)
    }

    #[inline(always)]
    pub fn swap(atomic: &AtomicBool, val: bool, order: Ordering) -> bool {
        atomic.swap(val, order)
    }
}
