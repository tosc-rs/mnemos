use crate::{Name, Word, WordFunc};
use core::alloc::Layout;
use core::ptr::addr_of_mut;
use core::ptr::NonNull;

// Starting FORTH: page 220
#[repr(C)]
pub struct DictionaryEntry {
    /// Precedence bit, length, and text characters
    /// Precedence bit is used to determine if it runs at compile or run time
    pub(crate) name: Name,
    /// Link field, points back to the previous entry
    pub(crate) link: Option<NonNull<DictionaryEntry>>,

    // HEAD ^
    // ------
    // BODY v
    /// Next is the "code pointer." The address contained in this
    /// pointer is what distinguishes a variable from a constant or a
    /// colon definition. It is the address of the instruction that is
    /// executed first when the particular type of word is executed.
    /// For example, in the case of a variable, the pointer points to code
    /// that pushes the address of the variable onto the stack.
    ///
    /// In the case of a constant, the pointer points to code that pushes the
    /// contents of the constant onto the stack. In the case of a colon
    /// definition, the pointer points to code that executes the rest of
    /// the words in the colon definition.
    ///
    /// The code that is pointed to is called the "run-time code"
    /// because it's used when a word of that type is executed (not when
    /// a word of that type is defined or compiled).
    pub(crate) code_pointer: WordFunc<'static, 'static>,

    /// data OR an array of compiled code.
    /// the first word is the "p(arameter)fa" or "c(ode)fa"
    pub(crate) parameter_field: [Word; 0],
}

pub struct DictionaryBump {
    pub(crate) start: *mut u8,
    pub(crate) cur: *mut u8,
    pub(crate) end: *mut u8,
}

impl DictionaryEntry {
    // Hmm, I probably won't ever actually "know" how many items I have,
    // since the actual editor will be more... dynamic than that.
    pub unsafe fn layout_for_arr(ct: usize) -> Layout {
        let layout_me = Layout::new::<Self>();
        let arr_size = core::mem::size_of::<Word>() * ct;
        let size = layout_me.size() + arr_size;
        Layout::from_size_align_unchecked(size, layout_me.align())
    }

    // TODO: This might be more sound if I make this part of the "find" function
    pub unsafe fn get_run<'a, 'b>(this: NonNull<Self>) -> (WordFunc<'a, 'b>, NonNull<Word>) {
        let de: &DictionaryEntry = this.as_ref();

        let wf: WordFunc<'static, 'static> = de.code_pointer;
        let wf: WordFunc<'a, 'b> = core::mem::transmute(wf);
        let cfa = DictionaryEntry::pfa(this);
        (wf, cfa)
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

    pub fn bump_u8(&mut self) -> Option<NonNull<u8>> {
        if self.cur >= self.end {
            None
        } else {
            let ptr = self.cur;
            self.cur = self.cur.wrapping_add(1);
            Some(unsafe { NonNull::new_unchecked(ptr) })
        }
    }

    pub fn bump<T: Sized>(&mut self) -> Option<NonNull<T>> {
        let offset = self.cur.align_offset(Layout::new::<T>().align());
        let align_cur = self.cur.wrapping_add(offset);
        let new_cur = align_cur.wrapping_add(Layout::new::<T>().size());

        if new_cur > self.end {
            None
        } else {
            self.cur = new_cur;
            Some(unsafe { NonNull::new_unchecked(align_cur.cast()) })
        }
    }

    pub fn bump_write<T: Sized>(&mut self, val: T) -> Result<(), ()> {
        let nnt = self.bump::<T>().ok_or(())?;
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
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        ptr::{addr_of_mut, NonNull},
    };

    use crate::{
        dictionary::{DictionaryBump, DictionaryEntry},
        test::LeakBox,
        Fif, Mode, Name, Word,
    };

    #[test]
    fn do_a_bump() {
        let payload: LeakBox<u8, 256> = LeakBox::new();

        let mut bump = DictionaryBump::new(payload.ptr(), payload.len());

        // Be annoying
        let _b = bump.bump_u8().unwrap();

        // ALLOT 10
        let d = bump.bump::<DictionaryEntry>().unwrap();
        assert_eq!(
            d.as_ptr()
                .align_offset(Layout::new::<DictionaryEntry>().align()),
            0
        );

        let walign = Layout::new::<DictionaryEntry>().align();
        for _w in 0..10 {
            let w = bump.bump::<Word>().unwrap();
            assert_eq!(w.as_ptr().align_offset(walign), 0);
        }
    }

    #[test]
    fn linked_list() {
        fn undefined(_fif: Fif<'_, '_>, _cfa: *mut Word) -> Result<(), ()> {
            #[cfg(test)]
            panic!("WHAT IS THIS EVEN");
            #[allow(unreachable_code)]
            Err(())
        }

        let layout_10 = unsafe { DictionaryEntry::layout_for_arr(10) };
        let node_a: NonNull<DictionaryEntry> =
            unsafe { NonNull::new(System.alloc(layout_10).cast()).unwrap() };

        unsafe {
            let nap = node_a.as_ptr();

            addr_of_mut!((*nap).name).write(Name::new_from_bstr(Mode::Run, b"hello"));
            addr_of_mut!((*nap).link).write(None);
            addr_of_mut!((*nap).code_pointer).write(undefined);

            for i in 0..10 {
                DictionaryEntry::pfa(node_a)
                    .as_ptr()
                    .add(i)
                    .write(Word::data(i as u32));
            }
        }
    }
}
