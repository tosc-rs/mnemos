use core::mem::MaybeUninit;

use crate::cfa::ShortWord;

// Use a union so that things work on both 32- and 64-bit systems,
// so the *data* is always 32 bits, but the pointer is whatever the
// native word size is.
#[repr(C)]
#[derive(Copy, Clone)]
pub union Word {
    pub data: i32,
    pub ptr: *mut (),
    pub hdr: ShortWord,
}

impl Word {
    #[inline]
    pub fn data(data: i32) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            mu_word.as_mut_ptr().cast::<i32>().write(data);
            mu_word.assume_init()
        }
    }

    #[inline]
    pub fn ptr<T>(ptr: *mut T) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            mu_word.as_mut_ptr().cast::<*mut T>().write(ptr);
            mu_word.assume_init()
        }
    }

    #[inline]
    pub fn hdr(hdr: ShortWord) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            mu_word.as_mut_ptr().cast::<ShortWord>().write(hdr);
            mu_word.assume_init()
        }
    }
}
