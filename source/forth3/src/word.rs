use core::{fmt::Debug, mem::MaybeUninit, ptr::addr_of_mut};

use crate::ReplaceErr;

// Use a union so that things work on both 32- and 64-bit systems,
// so the *data* is always 32 bits, but the pointer is whatever the
// native word size is.
#[repr(C)]
#[derive(Copy, Clone)]
pub union Word {
    pub data: i32,
    #[cfg(feature = "floats")]
    pub float: f32,
    pub ptr_data: isize,
    pub ptr: *mut (),
}

impl Debug for Word {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe { self.ptr.fmt(f) }
    }
}

impl PartialEq for Word {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.ptr.eq(&other.ptr) }
    }
}

impl TryFrom<usize> for Word {
    type Error = crate::Error;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        let val = i32::try_from(value).replace_err(crate::Error::UsizeToWordInvalid(value))?;
        Ok(Word::data(val))
    }
}

impl TryInto<usize> for Word {
    type Error = crate::Error;

    fn try_into(self) -> Result<usize, Self::Error> {
        let val = unsafe { self.data };
        usize::try_from(val).replace_err(crate::Error::WordToUsizeInvalid(val))
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

    #[cfg(feature = "floats")]
    #[inline]
    pub fn float(f: f32) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            addr_of_mut!((*mu_word.as_mut_ptr()).float).write(f);
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

    #[inline]
    pub fn ptr_data(ptr_data: isize) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            addr_of_mut!((*mu_word.as_mut_ptr()).ptr_data).write(ptr_data);
            mu_word.assume_init()
        }
    }
}
