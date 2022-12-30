use core::fmt::Debug;
use core::mem::MaybeUninit;
use core::ptr::{addr_of_mut, NonNull};

use crate::dictionary::{BuiltinEntry, DictionaryEntry};

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

impl TryFrom<usize> for Word {
    type Error = crate::Error;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        let val = i32::try_from(value).map_err(|_| crate::Error::UsizeToWordInvalid(value))?;
        Ok(Word::data(val))
    }
}

impl TryInto<usize> for Word {
    type Error = crate::Error;

    fn try_into(self) -> Result<usize, Self::Error> {
        let val = unsafe { self.data };
        usize::try_from(val).map_err(|_| crate::Error::WordToUsizeInvalid(val))
    }
}

pub enum PtrKind<T: 'static> {
    DictWord(NonNull<DictionaryEntry<T>>),
    BuiltinWord(NonNull<BuiltinEntry<T>>),
    Unknown(*mut ()),
}

impl Word {
    #[inline]
    pub unsafe fn as_kinded_ptr<T: 'static>(&self) -> PtrKind<T> {
        let ptr = self.ptr;
        let val = (ptr as usize) & 0b11;
        match val {
            1 => PtrKind::DictWord(NonNull::new_unchecked(ptr.cast::<u8>().offset(-1).cast())),
            2 => PtrKind::BuiltinWord(NonNull::new_unchecked(ptr.cast::<u8>().offset(-2).cast())),
            _ => PtrKind::Unknown(ptr),
        }
    }

    #[inline]
    pub fn data(data: i32) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            addr_of_mut!((*mu_word.as_mut_ptr()).data).write(data);
            mu_word.assume_init()
        }
    }

    #[inline]
    pub fn kinded_ptr<T: 'static>(ptr: PtrKind<T>) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        let ptr: *mut () = match ptr {
            PtrKind::DictWord(p) => p.cast::<u8>().as_ptr().wrapping_add(1).cast(),
            PtrKind::BuiltinWord(p) => p.cast::<u8>().as_ptr().wrapping_add(2).cast(),
            PtrKind::Unknown(p) => p,
        };
        unsafe {
            addr_of_mut!((*mu_word.as_mut_ptr()).ptr).write(ptr);
            mu_word.assume_init()
        }
    }

    // #[inline]
    // pub fn ptr<T>(ptr: *mut T) -> Self {
    //     let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
    //     unsafe {
    //         addr_of_mut!((*mu_word.as_mut_ptr()).ptr).write(ptr.cast());
    //         mu_word.assume_init()
    //     }
    // }
}
