use core::ptr::NonNull;
use core::{alloc::Layout, ptr::addr_of_mut, str::SplitWhitespace};
use core::str::FromStr;
use std::ptr::null_mut;

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
    pub const fn new_from_arr(mode: Mode, len: usize, arr: [u8; 31]) -> Self {
        assert!(len <= 31);
        let prec_len = match mode {
            Mode::Run => len as u8,
            Mode::Compile => (len as u8) | 0x80,
        };
        let mut i = 0;
        while i < len {
            assert!(arr[i].is_ascii());
            i += 1;
        }
        Self {
            prec_len,
            name: arr,
        }
    }

    pub fn new_from_bstr(mode: Mode, bstr: &[u8]) -> Self {
        let len = bstr.len().min(31);
        let prec_len = match mode {
            Mode::Run => len as u8,
            Mode::Compile => (len as u8) | 0x80,
        };

        let mut new = Name {
            prec_len,
            name: [0u8; 31],
        };
        new.name[..len].copy_from_slice(&bstr[..len]);

        // TODO: Smarter way to make sure this is a str?
        debug_assert!({
            (&new.name[..len]).iter().all(|b| b.is_ascii())
        });

        new
    }
}

static ONE: Bide = Bide { de: DictionaryEntry {
        name: Name::new_from_arr(Mode::Run, 5, *b"hello                          "),
        link: None,
        code_pointer: Fif::undefined,
        parameter_field: [],
    }};
static TWO: Bide = Bide { de: DictionaryEntry {
        name: Name::new_from_arr(Mode::Run, 5, *b"hello                          "),
        link: Some(unsafe {
            NonNull::new_unchecked(
                ((&ONE.de) as *const DictionaryEntry) as *mut DictionaryEntry
            )
        }),
        code_pointer: Fif::undefined,
        parameter_field: [],
    }};

unsafe impl Sync for Bide { }

#[repr(transparent)]
struct Bide {
    de: DictionaryEntry,
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
    code_pointer: WordFunc<'static, 'static>,

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

    // TODO: This might be more sound if I make this part of the "find" function
    pub unsafe fn get_run<'a, 'b>(this: NonNull<Self>) -> (WordFunc<'a, 'b>, NonNull<Word>) {
        let wf: WordFunc<'static, 'static> = this.as_ref().code_pointer;
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

// fn(Fif<'a, 'b>, *mut Word) -> Result<(), ()>


pub struct DictionaryBump {
    start: *mut u8,
    cur: *mut u8,
    end: *mut u8,
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
            Some(unsafe { NonNull::new_unchecked(new_cur.cast()) })
        }
    }
}

pub struct WordStrBuf {
    start: *mut u8,
    cur: *mut u8,
    end: *mut u8,
}

impl WordStrBuf {
    pub fn new(bottom: *mut u8, size: usize) -> Self {
        let end = bottom.wrapping_add(size);
        debug_assert!(end >= bottom);
        Self {
            end,
            start: bottom,
            cur: end,
        }
    }

    // fn remaining(&self) -> &str {
    //     unsafe {
    //         let size = (self.end as usize) - (self.cur as usize);
    //         let rem_sli = core::slice::from_raw_parts(self.cur, size);
    //         let rem_str = core::str::from_utf8_unchecked(rem_sli);
    //         rem_str
    //     }
    // }
    #[inline]
    fn capacity(&self) -> usize {
        (self.end as usize) - (self.start as usize)
    }

    pub fn fill(&mut self, input: &str) -> Result<(), ()> {
        let ilen = input.len();
        let cap = self.capacity();
        if ilen > cap {
            return Err(());
        }
        if !input.is_ascii() {
            // TODO: Do I care about this?
            return Err(());
        }
        unsafe {
            let istart = input.as_bytes().as_ptr();
            for i in 0..ilen {
                self.start.add(i).write((*istart.add(i)).to_ascii_lowercase());
            }
            core::ptr::write_bytes(
                self.start.add(ilen),
                b' ',
                cap - ilen,
            );
        }
        self.cur = self.start;
        Ok(())
    }

    pub fn next_word(&mut self) -> Option<&str> {
        // Find the start, skipping any ASCII whitespace
        let start = loop {
            if self.cur == self.end {
                return None;
            }
            if !unsafe { *self.cur }.is_ascii_whitespace() {
                break self.cur;
            }
            self.cur = self.cur.wrapping_add(1);
        };
        // Find the end, either the first ASCII whitespace, or the end of the buffer
        // This is ONE PAST the last character
        let end = loop {
            if self.cur == self.end {
                break self.end;
            }
            if unsafe { *self.cur }.is_ascii_whitespace() {
                break self.cur;
            }
            self.cur = self.cur.wrapping_add(1);
        };
        let size = (end as usize) - (start as usize);
        Some(unsafe {
            let u8_sli = core::slice::from_raw_parts(start, size);
            core::str::from_utf8_unchecked(u8_sli)
        })
    }
}

// Is this just context?
pub enum Mode {
    Run,
    Compile,
}

pub struct Forth {
    mode: Mode,
    data_stack: Stack,
    dict_alloc: DictionaryBump,
    run_dict_tail: Option<NonNull<DictionaryEntry>>,
    comp_dict_tail: Option<NonNull<DictionaryEntry>>,
}

pub struct Fif<'a, 'b> {
    forth: &'a mut Forth,
    input: &'b mut WordStrBuf,
}

impl<'a, 'b> Fif<'a, 'b> {
    pub fn undefined(self, _cfa: *mut Word) -> Result<(), ()> {
        panic!("WHAT IS THIS EVEN");
    }

    pub fn pop_print(self, _cfa: *mut Word) -> Result<(), ()> {
        let a = self.forth.data_stack.pop().ok_or(())?;
        println!("{}", unsafe { a.data });
        Ok(())
    }

    pub fn add(self, _cfa: *mut Word) -> Result<(), ()> {
        let a = self.forth.data_stack.pop().ok_or(())?;
        let b = self.forth.data_stack.pop().ok_or(())?;
        self.forth.data_stack.push(Word::data(unsafe {
            a.data.wrapping_add(b.data)
        }))
    }

    pub fn literal(self, _cfa: *mut Word) -> Result<(), ()> {
        // TODO: Do I only use this as a sentinel?
        Err(())
    }

    pub fn colon(self, cfa: *mut Word) -> Result<(), ()> {
        match self.forth.mode {
            Mode::Run => todo!(),
            Mode::Compile => {
                let name = self.input.next_word().ok_or(())?;
                let name = Name::new_from_bstr(Mode::Run, name.as_bytes());

                // TODO, I could check that there is at least a `;` here,
                // but that ignores any other errors. Let's plough ahead,
                // at the risk we "leak" dictionary memory in the case of
                // a bad compile. Later: we can figure out how to "unwind"
                // this and reclaim the allocated memory

                let word_base = self
                    .forth
                    .dict_alloc
                    .bump::<DictionaryEntry>()
                    .ok_or(())?;

                unsafe {
                    word_base.as_ptr().write(DictionaryEntry {
                        name,
                        // Don't link until we know we have a "good" entry!
                        link: None,
                        code_pointer: Fif::colon,
                        parameter_field: [],
                    });
                }

                // Rather than having an "exit" word, I'll prepend the
                // cfa array with a length field (NOT including the length
                // itself).
                let len: &mut u32 = {
                    let len_word = self
                        .forth
                        .dict_alloc
                        .bump::<Word>()
                        .ok_or(())?;
                    unsafe {
                        len_word.as_ptr().write(Word::data(0));
                        &mut (*len_word.as_ptr()).data
                    }
                };

                let mut semicolon = false;

                while let Some(word) = self.input.next_word() {
                    match self.forth.lookup(word)? {
                        Lookup::Builtin { func } => todo!(),
                        Lookup::Dict { func, cfa } => todo!(),
                        Lookup::Literal { val } => todo!(),
                    }
                }
                // Link to run dict
                // (&mut *word_base.as_ptr()).link = self.forth.run_dict_tail.take();
                // self.forth.run_dict_tail = Some(word_base);
            },
        }

        todo!()
    }
}

pub enum Lookup<'a, 'b> {
    Builtin {
        func: WordFunc<'a, 'b>,
    },
    Dict {
        func: WordFunc<'a, 'b>,
        cfa: NonNull<Word>,
    },
    Literal {
        val: u32,
    }
}

type WordFunc<'a, 'b> = fn(Fif<'a, 'b>, *mut Word) -> Result<(), ()>;
// !!!!!!!!
// ! TODO !
// !!!!!!!!
//
// `Forth` shouldn't hold it's own input buffer. It should be one level up,
// so we can irrefutably bind the forth context and input buffers with
// different lifetimes.
impl Forth {

    pub unsafe fn new(
        stack_buf: (*mut Word, usize),
        dict_buf: (*mut u8, usize),
    ) -> Self {
        let data_stack = Stack::new(stack_buf.0, stack_buf.1);
        let dict_alloc = DictionaryBump::new(dict_buf.0, dict_buf.1);
        Self {
            mode: Mode::Run,
            data_stack,
            dict_alloc,
            run_dict_tail: None,
            comp_dict_tail: None,
        }
    }

    fn parse_num(word: &str) -> Option<u32> {
        u32::from_str(word).ok()
    }

    fn find_in_dict<'a>(&self, _word: &'a str) -> Option<NonNull<DictionaryEntry>> {
        None
    }

    fn find_builtin<'a, 'b>(word: &'b str) -> Option<WordFunc<'a, 'b>> {
        Some(match word {
            "add" => Fif::add,
            "." => Fif::pop_print,
            _ => return None,
        })
    }

    pub fn lookup<'a>(&self, word: &'a str) -> Result<Lookup<'_, 'a>, ()> {
        if let Some(func) = Self::find_builtin(word) {
            Ok(Lookup::Builtin { func })
        } else if let Some(entry) = self.find_in_dict(word) {
            let (func, cfa) = unsafe { DictionaryEntry::get_run(entry) };
            Ok(Lookup::Dict { func, cfa })
        } else if let Some(val) = Self::parse_num(word) {
            Ok(Lookup::Literal { val })
        } else {
            Err(())
        }
    }

    pub fn process_line<'a>(&mut self, line: &'a mut WordStrBuf) -> Result<(), ()> {
        while let Some(word) = line.next_word() {
            match self.lookup(word)? {
                Lookup::Builtin { func } => func(Fif { forth: self, input: line }, null_mut()),
                Lookup::Dict { func, cfa } => func(Fif { forth: self, input: line }, cfa.as_ptr()),
                Lookup::Literal { val } => {
                    self.data_stack.push(Word::data(val))
                },
            }?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod test {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        mem::MaybeUninit,
        ptr::{addr_of_mut, NonNull},
    };

    use crate::{DictionaryBump, DictionaryEntry, Name, Stack, Word, Forth, WordStrBuf, Fif, Mode};

    #[test]
    fn forth() {
        let payload_stack: *mut Word = Box::leak(Box::new(MaybeUninit::<[Word; 256]>::uninit()))
            .as_mut_ptr()
            .cast();
        let input_buf: *mut u8 = Box::leak(Box::new(MaybeUninit::<[u8; 256]>::uninit()))
            .as_mut_ptr()
            .cast();
        let dict_buf: *mut u8 = Box::leak(Box::new(MaybeUninit::<[u8; 512]>::uninit()))
            .as_mut_ptr()
            .cast();

        let mut input = WordStrBuf::new(input_buf, 256);
        let mut forth = unsafe { Forth::new(
            (payload_stack, 256),
            (dict_buf, 512),
        ) };
        input.fill("2 3 add .").unwrap();
        forth.process_line(&mut input).unwrap();
        panic!();
    }

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

            addr_of_mut!((*nap).name).write(Name::new_from_bstr(Mode::Run, b"hello"));
            addr_of_mut!((*nap).link).write(None);
            addr_of_mut!((*nap).code_pointer).write(Fif::undefined);

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
