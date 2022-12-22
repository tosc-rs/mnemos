use core::ptr::NonNull;
use std::{alloc::Layout, ptr::addr_of_mut};

// SAFETY: James needs to audit basically every use of `wrapping_x` on a pointer type.

pub struct Something;
pub struct SomethingUp;
pub struct SomethingDown;
pub struct SomethingVariablySized;

// Use a union so that things work on both 32- and 64-bit systems,
// so the *data* is always 32 bits, but the pointer is whatever the
// native word size is.
pub union Word {
    data: u32,
    ptr: *mut (),
}

impl Word {
    #[inline]
    fn data(data: u32) -> Self {
        Word { data }
    }

    #[inline]
    fn ptr<T>(ptr: *mut T) -> Self {
        Word { ptr: ptr.cast() }
    }
}

const CONTEXTS: usize = 3; // forth, editor, assembler (any others?)
const CONTEXT_IDX_FORTH: usize = 0;
const CONTEXT_IDX_EDITOR: usize = 1;
const CONTEXT_IDX_ASSEMBLER: usize = 2;

// Starting FORTH: page 231
// This structure probably won't concretely exist
pub struct Everything {
    /// Precompiled forth words
    builtin_words: Something,
    /// Variables that affect the system
    system_variables: Something,
    /// Option (also compiled?) forth words
    elective_definitions: Something,

    contexts: [Option<NonNull<DictionaryEntry>>; CONTEXTS],
    context_idx: usize,

    // /// hmm
    // pad: Something, // technically this lives at the top of the user dict?
    /// Main stack
    parameter_stack: Stack,
    /// Input scratch buffer
    input_msg_buffer: SomethingUp,
    /// Return (secondary) stack
    return_stack: Stack,
    /// Special User Variables
    user_variables: Something,
    /// Used for paging from disk
    block_buffers: Something,
}

pub struct Name {
    prec_len: u8,
    name: [u8; 31],
}

impl Name {
    pub fn new_from_bstr(precidence: bool, bstr: &[u8]) -> Self {
        let len = bstr.len().min(31);
        let prec_len = if precidence {
            (len as u8) | 0x80
        } else {
            len as u8
        };

        let mut new = Name {
            prec_len,
            name: [0u8; 31],
        };
        new.name[..len].copy_from_slice(&bstr[..len]);
        new
    }
}

// Starting FORTH: page 220
#[repr(C)]
pub struct DictionaryEntry {
    /// Precedence bit, length, and text characters
    /// Precedence bit is used to determine if it runs at compile or run time
    name: Name,
    /// Link field, points back to the previous entry
    link: Option<NonNull<DictionaryEntry>>,

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
    code_pointer: fn(Something) -> Something,

    /// data OR an array of compiled code.
    /// the first word is the "p(arameter)fa" or "c(ode)fa"
    parameter_field: [Word; 0],
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

    pub unsafe fn pfa(this: NonNull<Self>) -> NonNull<Word> {
        let ptr = this.as_ptr();
        let pfp: *mut [Word; 0] = addr_of_mut!((*ptr).parameter_field);
        NonNull::new_unchecked(pfp.cast::<Word>())
    }
}

pub struct Stack {
    top: *mut Word,
    cur: *mut Word,
    bot: *mut Word,
}

impl Stack {
    pub fn new(bottom: *mut Word, words: usize) -> Self {
        let top = bottom.wrapping_add(words);
        debug_assert!(top >= bottom);
        Self {
            top,
            bot: bottom,
            cur: top,
        }
    }

    #[inline]
    pub fn push(&mut self, word: Word) -> Result<(), ()> {
        let next_cur = self.cur.wrapping_sub(1);
        if next_cur < self.bot {
            return Err(());
        }
        self.cur = next_cur;
        unsafe {
            self.cur.write(word);
        }
        Ok(())
    }

    #[inline]
    pub fn pop(&mut self) -> Option<Word> {
        let next_cur = self.cur.wrapping_add(1);
        if next_cur > self.top {
            return None;
        }
        let val = unsafe { self.cur.read() };
        self.cur = next_cur;
        Some(val)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.cur = self.top;
    }
}

fn undefined(_: Something) -> Something {
    panic!("WHAT IS THIS EVEN");
}

pub struct DictionaryBump {
    start: *mut u8,
    cur: *mut u8,
    end: *mut u8,
}

impl DictionaryBump {
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
            Some(unsafe { NonNull::new_unchecked(new_cur.cast()) })
        }
    }
}

#[cfg(test)]
pub mod test {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        mem::MaybeUninit,
        ptr::{addr_of_mut, NonNull},
    };

    use crate::{undefined, DictionaryBump, DictionaryEntry, Name, Stack, Word};

    #[test]
    fn do_a_bump() {
        let payload: *mut u8 = Box::leak(Box::new(MaybeUninit::<[u8; 256]>::uninit()))
            .as_mut_ptr()
            .cast();

        let mut bump = DictionaryBump {
            start: payload,
            cur: payload,
            end: payload.wrapping_add(256),
        };

        // Be annoying
        let b = bump.bump_u8().unwrap();

        // ALLOT 10
        let d = bump.bump::<DictionaryEntry>().unwrap();
        assert_eq!(d.as_ptr().align_offset(Layout::new::<DictionaryEntry>().align()), 0);

        let walign = Layout::new::<DictionaryEntry>().align();
        for w in 0..10 {
            let w = bump.bump::<Word>().unwrap();
            assert_eq!(w.as_ptr().align_offset(walign), 0);
        }

        unsafe {
            let _ = Box::<MaybeUninit<[u8; 256]>>::from_raw(payload.cast());
        }
    }

    #[test]
    fn linked_list() {
        let layout_10 = unsafe { DictionaryEntry::layout_for_arr(10) };
        let node_a: NonNull<DictionaryEntry> =
            unsafe { NonNull::new(System.alloc(layout_10).cast()).unwrap() };

        unsafe {
            let nap = node_a.as_ptr();

            addr_of_mut!((*nap).name).write(Name::new_from_bstr(true, b"hello"));
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

    #[test]
    fn stack() {
        const ITEMS: usize = 16;
        let payload = Box::leak(Box::new(MaybeUninit::<[Word; ITEMS]>::uninit()))
            .as_mut_ptr()
            .cast();

        let mut stack = Stack::new(payload, ITEMS);

        for _ in 0..3 {
            for i in 0..(ITEMS as u32) {
                assert!(stack.push(Word::data(i)).is_ok());
            }
            assert!(stack.push(Word::data(100)).is_err());
            for i in (0..(ITEMS as u32)).rev() {
                assert_eq!(unsafe { stack.pop().unwrap().data }, i);
            }
            assert!(stack.pop().is_none());
        }
        unsafe {
            let _ = Box::<MaybeUninit<[Word; ITEMS]>>::from_raw(payload.cast());
        }
    }
}
