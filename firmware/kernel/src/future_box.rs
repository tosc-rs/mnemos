// TODO: This will probably need to move into the Common library,
// or at least some version of it.

use core::{sync::atomic::{AtomicU8, AtomicBool, Ordering}, ops::{Deref, DerefMut}, ptr::null_mut};

use crate::alloc::{HeapBox, HeapArray};

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
}

// ------------------ | FUTURE BOX | ------------------------

// This gets leaked
#[repr(C)]
pub struct FutureBox<T> {
    // TODO: Should these fields be one atomic u32?

    // Current status. Should only be updated by the holder of
    // the exclusive token
    status: AtomicU8,

    // Reference count, including exclusive and shared handles
    refcnt: AtomicU8,

    // Is the exclusive handle taken?
    ex_taken: AtomicBool,

    // TODO: This is a HeapBox<T>.
    payload: *mut T,
}

impl<T> Drop for FutureBoxExHdl<T> {
    fn drop(&mut self) {
        let drop_fb = {
            let fb = unsafe { &*self.fb };
            let pre_refs = fb.refcnt.fetch_sub(1, Ordering::SeqCst);
            fb.ex_taken.store(false, Ordering::SeqCst);
            debug_assert!(pre_refs != 0);
            pre_refs <= 1
        };

        // Split off, to avoid reference to self.fb being live
        // SAFETY: This arm only executes if we were the LAST handle to know
        // about this futurebox.
        if drop_fb {
            // We are responsible for dropping the payload, and the futurebox
            if self.payload != null_mut() {
                let _ = unsafe { HeapBox::from_leaked(self.payload) };
            }
            let _ = unsafe { HeapBox::from_leaked(self.fb) };
        }
    }
}

// This represents shared access to the FutureBox, and
// exclusive access to the payload
pub struct FutureBoxExHdl<T> {
    fb: *mut FutureBox<T>,
    // Store the payload handle here, so we don't have to double deref
    payload: *mut T,
}

impl<T> Deref for FutureBoxExHdl<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: We have exclusive access for as long as this handle exists
        unsafe {
            &*self.payload
        }
    }
}

impl<T> DerefMut for FutureBoxExHdl<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We have exclusive access for as long as this handle exists
        unsafe {
            &mut *self.payload
        }
    }
}

// This represents shared access to the FutureBox, and
// NO access to the payload
pub struct FutureBoxPendHdl<T> {
    fb: *mut FutureBox<T>,
    awaiting: u8,
}

impl<T> FutureBoxPendHdl<T> {
    pub fn is_complete(&self) -> Result<bool, ()> {
        let fb = unsafe { &*self.fb };
        match fb.status.load(Ordering::SeqCst) {
            status::COMPLETED => Ok(true),
            status::ERROR => Err(()),
            _ => Ok(false),
        }
    }

    pub fn try_upgrade(&self) -> Result<Option<FutureBoxExHdl<T>>, ()> {
        let fb = unsafe { &*self.fb };
        let was_ex = fb.ex_taken.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
        match was_ex {
            Ok(_) => {
                // We have exclusive access, see if we are in the right mode
                match fb.status.load(Ordering::SeqCst) {
                    status::ERROR => {
                        // It's never gunna work out...
                        fb.ex_taken.store(false, Ordering::SeqCst);
                        return Err(());
                    }
                    n if n == self.awaiting => {
                        // Yup!
                        let fbeh = FutureBoxExHdl {
                            fb: self.fb,
                            payload: fb.payload,
                        };
                        fb.refcnt.fetch_add(1, Ordering::SeqCst);
                        Ok(Some(fbeh))
                    }
                    _ => {
                        // Nope. Release exclusive access
                        fb.ex_taken.store(false, Ordering::SeqCst);
                        Ok(None)
                    }
                }
            }
            Err(_) => {
                // It failed. Someone else has exclusive access.
                return Ok(None);
            }
        }
    }
}

// ------------------ | FUTURE ARRAY | ------------------------

// This gets leaked
#[repr(C)]
pub struct FutureArray<T> {
    // TODO: Should these fields be one atomic u32?

    // Current status. Should only be updated by the holder of
    // the exclusive token
    status: AtomicU8,

    // Reference count, including exclusive and shared handles
    refcnt: AtomicU8,

    // Is the exclusive handle taken?
    ex_taken: AtomicBool,

    // TODO: This is a HeapArray<T>.
    payload: *mut T,
    count: usize,
}

impl<T> Drop for FutureArrayExHdl<T> {
    fn drop(&mut self) {
        let drop_fb = {
            let fb = unsafe { &*self.fb };
            let pre_refs = fb.refcnt.fetch_sub(1, Ordering::SeqCst);
            fb.ex_taken.store(false, Ordering::SeqCst);
            debug_assert!(pre_refs != 0);
            pre_refs <= 1
        };

        // Split off, to avoid reference to self.fb being live
        // SAFETY: This arm only executes if we were the LAST handle to know
        // about this FutureArray.
        if drop_fb {
            // We are responsible for dropping the payload, and the FutureArray
            if self.payload != null_mut() {
                let _ = unsafe { HeapArray::from_leaked(self.payload, self.count) };
            }
            let _ = unsafe { HeapBox::from_leaked(self.fb) };
        }
    }
}

// This represents shared access to the FutureArray, and
// exclusive access to the payload
pub struct FutureArrayExHdl<T> {
    fb: *mut FutureArray<T>,
    // Store the payload handle here, so we don't have to double deref
    payload: *mut T,
    count: usize,
}

impl<T> Deref for FutureArrayExHdl<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        // SAFETY: We have exclusive access for as long as this handle exists
        unsafe {
            core::slice::from_raw_parts(self.payload, self.count)
        }
    }
}

impl<T> DerefMut for FutureArrayExHdl<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We have exclusive access for as long as this handle exists
        unsafe {
            core::slice::from_raw_parts_mut(self.payload, self.count)
        }
    }
}

// This represents shared access to the FutureArray, and
// NO access to the payload
pub struct FutureArrayPendHdl<T> {
    fb: *mut FutureArray<T>,
    awaiting: u8,
}

impl<T> FutureArrayPendHdl<T> {
    pub fn is_complete(&self) -> Result<bool, ()> {
        let fb = unsafe { &*self.fb };
        match fb.status.load(Ordering::SeqCst) {
            status::COMPLETED => Ok(true),
            status::ERROR => Err(()),
            _ => Ok(false),
        }
    }

    pub fn try_upgrade(&self) -> Result<Option<FutureArrayExHdl<T>>, ()> {
        let fb = unsafe { &*self.fb };
        let was_ex = fb.ex_taken.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
        match was_ex {
            Ok(_) => {
                // We have exclusive access, see if we are in the right mode
                match fb.status.load(Ordering::SeqCst) {
                    status::ERROR => {
                        // It's never gunna work out...
                        fb.ex_taken.store(false, Ordering::SeqCst);
                        return Err(());
                    }
                    n if n == self.awaiting => {
                        // Yup!
                        let fbeh = FutureArrayExHdl {
                            fb: self.fb,
                            payload: fb.payload,
                            count: fb.count,
                        };
                        fb.refcnt.fetch_add(1, Ordering::SeqCst);
                        Ok(Some(fbeh))
                    }
                    _ => {
                        // Nope. Release exclusive access
                        fb.ex_taken.store(false, Ordering::SeqCst);
                        Ok(None)
                    }
                }
            }
            Err(_) => {
                // It failed. Someone else has exclusive access.
                return Ok(None);
            }
        }
    }
}
