use crate::fastr::FaStr;
use crate::{Word, WordFunc};
use core::alloc::Layout;
use core::ptr::addr_of_mut;
use core::ptr::NonNull;

#[derive(Debug, PartialEq)]
pub enum BumpError {
    OutOfMemory,
    CantAllocUtf8,
}

#[repr(u16)]
pub enum EntryKind {
    StaticBuiltin,
    RuntimeBuiltin,
    Dictionary,
}

#[repr(C)]
pub struct EntryHeader<T: 'static> {
    pub func: WordFunc<T>,
    pub name: FaStr,
    pub kind: EntryKind, // todo
    pub len: u16,
}


#[repr(C)]
pub struct BuiltinEntry<T: 'static> {
    pub hdr: EntryHeader<T>,
}

// Starting FORTH: page 220
#[repr(C)]
pub struct DictionaryEntry<T: 'static> {
    pub hdr: EntryHeader<T>,

    /// Link field, points back to the previous entry
    pub(crate) link: Option<NonNull<DictionaryEntry<T>>>,

    /// data OR an array of compiled code.
    /// the first word is the "p(arameter)fa" or "c(ode)fa"
    pub(crate) parameter_field: [Word; 0],
}

pub struct DictionaryBump {
    pub(crate) start: *mut u8,
    pub(crate) cur: *mut u8,
    pub(crate) end: *mut u8,
}

impl<T: 'static> DictionaryEntry<T> {
    // Hmm, I probably won't ever actually "know" how many items I have,
    // since the actual editor will be more... dynamic than that.
    pub unsafe fn layout_for_arr(ct: usize) -> Layout {
        let layout_me = Layout::new::<Self>();
        let arr_size = core::mem::size_of::<Word>() * ct;
        let size = layout_me.size() + arr_size;
        Layout::from_size_align_unchecked(size, layout_me.align())
    }

    pub unsafe fn pfa(this: NonNull<Self>) -> NonNull<Word> {
        let ptr = this.as_ptr();
        let pfp: *mut [Word; 0] = addr_of_mut!((*ptr).parameter_field);
        NonNull::new_unchecked(pfp.cast::<Word>())
    }
}

impl DictionaryBump {
    pub fn new(bottom: *mut u8, size: usize) -> Self {
        let end = bottom.wrapping_add(size);
        debug_assert!(end >= bottom);
        Self {
            end,
            start: bottom,
            cur: bottom,
        }
    }

    pub fn bump_str(&mut self, s: &str) -> Result<FaStr, BumpError> {
        debug_assert!(!s.is_empty());

        let len = s.len().min(31);
        let astr = &s.as_bytes()[..len];

        if !astr.iter().all(|b| b.is_ascii()) {
            return Err(BumpError::CantAllocUtf8);
        }
        let stir = self.bump_u8s(len).ok_or(BumpError::OutOfMemory)?.as_ptr();
        for (i, ch) in astr.iter().enumerate() {
            unsafe {
                stir.add(i).write(ch.to_ascii_lowercase());
            }
        }
        unsafe { Ok(FaStr::new(stir, len)) }
    }

    pub fn bump_u8s(&mut self, n: usize) -> Option<NonNull<u8>> {
        if n == 0 {
            return None;
        }

        let req = self.cur.wrapping_add(n);

        if req > self.end {
            None
        } else {
            let ptr = self.cur;
            self.cur = req;
            Some(unsafe { NonNull::new_unchecked(ptr) })
        }
    }

    pub fn bump_u8(&mut self) -> Option<NonNull<u8>> {
        if self.cur >= self.end {
            None
        } else {
            let ptr = self.cur;
            self.cur = self.cur.wrapping_add(1);
            Some(unsafe { NonNull::new_unchecked(ptr) })
        }
    }

    pub fn bump<T: Sized>(&mut self) -> Result<NonNull<T>, BumpError> {
        let offset = self.cur.align_offset(Layout::new::<T>().align());
        let align_cur = self.cur.wrapping_add(offset);
        let new_cur = align_cur.wrapping_add(Layout::new::<T>().size());

        if new_cur > self.end {
            Err(BumpError::OutOfMemory)
        } else {
            self.cur = new_cur;
            Ok(unsafe { NonNull::new_unchecked(align_cur.cast()) })
        }
    }

    pub fn bump_write<T: Sized>(&mut self, val: T) -> Result<(), BumpError> {
        let nnt = self.bump::<T>()?;
        unsafe {
            nnt.as_ptr().write(val);
        }
        Ok(())
    }

    /// Is the given pointer within the dictionary range?
    pub fn contains(&self, ptr: *mut ()) -> bool {
        let pau = ptr as usize;
        let sau = self.start as usize;
        let eau = self.end as usize;
        (pau >= sau) && (pau < eau)
    }

    pub fn used(&self) -> usize {
        (self.cur as usize) - (self.start as usize)
    }
}

#[cfg(test)]
pub mod test {
    use std::alloc::Layout;
    use core::mem::size_of;

    use crate::{
        dictionary::{DictionaryBump, DictionaryEntry},
        leakbox::LeakBox,
        Word,
    };

    use super::EntryHeader;

    #[test]
    fn sizes() {
        assert_eq!(size_of::<EntryHeader<()>>(), 4 * size_of::<usize>());
    }

    #[test]
    fn do_a_bump() {
        let payload: LeakBox<u8> = LeakBox::new(256);

        let mut bump = DictionaryBump::new(payload.ptr(), payload.len());

        // Be annoying
        let _b = bump.bump_u8().unwrap();

        // ALLOT 10
        let d = bump.bump::<DictionaryEntry<()>>().unwrap();
        assert_eq!(
            d.as_ptr()
                .align_offset(Layout::new::<DictionaryEntry<()>>().align()),
            0
        );

        let walign = Layout::new::<DictionaryEntry<()>>().align();
        for _w in 0..10 {
            let w = bump.bump::<Word>().unwrap();
            assert_eq!(w.as_ptr().align_offset(walign), 0);
        }
    }
}
