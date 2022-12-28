use core::fmt::Debug;
use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

// Use a union so that things work on both 32- and 64-bit systems,
// so the *data* is always 32 bits, but the pointer is whatever the
// native word size is.
#[repr(C)]
#[derive(Copy, Clone)]
pub union Word {
    pub data: i32,
    pub ptr: *mut (),
}

impl Debug for Word {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe { self.ptr.fmt(f) }
    }
}

impl PartialEq for Word {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.ptr.eq(&other.ptr) }
    }
}

impl Word {
    #[inline]
    pub fn data(data: i32) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            addr_of_mut!((*mu_word.as_mut_ptr()).data).write(data);
            mu_word.assume_init()
        }
    }

    #[inline]
    pub fn ptr<T>(ptr: *mut T) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            addr_of_mut!((*mu_word.as_mut_ptr()).ptr).write(ptr.cast());
            mu_word.assume_init()
        }
    }
}
