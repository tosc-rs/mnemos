use core::alloc::Layout;
use core::sync::atomic::{AtomicU32, AtomicU8, AtomicBool, AtomicPtr};
use core::ops::Deref;
use serde::{Serialize, Deserialize};

#[repr(C)]
pub struct BoxBytes<const N: usize> {
    // when N == 0, we can use the size to
    // get the slice
    capacity: u32,
    len: u32,
    payload: [u8; N],
}

impl<const N: usize> Deref for BoxBytes<N> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.payload.as_slice()
    }
}

impl BoxBytes<0> {
    pub fn deref_dyn(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self.payload.as_ptr(),
                self.len as usize,
            )
        }
    }

    // TODO: I think this is right?
    pub fn layout(&self) -> Layout {
        let me = Layout::new::<BoxBytes<0>>();
        let me_align = me.align();
        let align_add = me_align - 1;
        // Round up to next
        // TODO: Replace with `next_multiple_of` once https://github.com/rust-lang/rust/issues/88581 lands
        let to_add = ((self.capacity as usize + align_add) / me_align) * me_align;
        unsafe {
            Layout::from_size_align_unchecked(
                me.size() + to_add,
                me.align(),
            )
        }
    }
}

impl<const N: usize> From<&'static BoxBytes<N>> for SysCallBoxBytes {
    fn from(other: &'static BoxBytes<N>) -> Self {
        Self {
            bb_ptr: (other as *const BoxBytes<N>) as usize as u32,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SysCallBoxBytes {
    bb_ptr: u32,
}

// This is the "stable future" item
#[repr(C)]
pub struct FutureBytes {
    refcnt: AtomicU32,
    payload: AtomicPtr<BoxBytes<0>>,
    status: AtomicU8,
    ex_taken: AtomicBool,
}

pub mod status {
    /// Kernel is working, and should be allowed exclusive access,
    /// if it doesn't have it already.
    pub const KERNEL_ACCESS: u8 = 0;

    /// Userspace is working, and should be allowed exclusive access,
    /// if it doesn't have it already.
    pub const USERSPACE_ACCESS: u8 = 1;

    /// The future has completed (on either side), but the payload
    /// is no longer accessible.
    pub const COMPLETED: u8 = 2;

    /// This future encountered an error, and will never reach the
    /// completed stage. The payload is no longer accessible.
    pub const ERROR: u8 = 3;

    /// Used to signify a handle that will only ever pend error or completed
    pub const INVALID: u8 = 4;
}

#[derive(Serialize, Deserialize)]
pub struct SysCallFutureBytes {
    fb_ptr: u32,
}

impl From<&'static FutureBytes> for SysCallFutureBytes {
    fn from(other: &'static FutureBytes) -> Self {
        Self {
            fb_ptr: (other as *const FutureBytes) as usize as u32,
        }
    }
}
