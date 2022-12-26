use core::ptr::NonNull;
use core::str::FromStr;
use core::{alloc::Layout, ptr::addr_of_mut, str::SplitWhitespace};
use std::mem::{transmute, MaybeUninit};
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
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            mu_word.as_mut_ptr().cast::<u32>().write(data);
            mu_word.assume_init()
        }
    }

    #[inline]
    fn ptr<T>(ptr: *mut T) -> Self {
        let mut mu_word: MaybeUninit<Word> = MaybeUninit::zeroed();
        unsafe {
            mu_word.as_mut_ptr().cast::<*mut T>().write(ptr);
            mu_word.assume_init()
        }
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
        debug_assert!({ (&new.name[..len]).iter().all(|b| b.is_ascii()) });

        new
    }

    pub fn as_str(&self) -> &str {
        let len = (self.prec_len & 0x7F) as usize;
        unsafe { core::str::from_utf8_unchecked(&self.name[..len]) }
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
            Some(unsafe { NonNull::new_unchecked(align_cur.cast()) })
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
                self.start
                    .add(i)
                    .write((*istart.add(i)).to_ascii_lowercase());
            }
            core::ptr::write_bytes(self.start.add(ilen), b' ', cap - ilen);
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
    // DON'T include `literal` here!
    const BUILTINS: &'static [(&'static str, WordFunc<'static, 'static>)] = &[
        ("add", Fif::add),
        (".", Fif::pop_print),
        (":", Fif::colon),
        ("(undefined)", Fif::undefined),
        ("(interpret)", Fif::interpret),
    ];

    pub fn undefined(self, _cfa: *mut Word) -> Result<(), ()> {
        #[cfg(test)]
        panic!("WHAT IS THIS EVEN");
        Err(())
    }

    pub fn pop_print(self, _cfa: *mut Word) -> Result<(), ()> {
        let a = self.forth.data_stack.pop().ok_or(())?;
        #[cfg(test)]
        print!("{} ", unsafe { a.data });
        Ok(())
    }

    pub fn add(self, _cfa: *mut Word) -> Result<(), ()> {
        let a = self.forth.data_stack.pop().ok_or(())?;
        let b = self.forth.data_stack.pop().ok_or(())?;
        self.forth
            .data_stack
            .push(Word::data(unsafe { a.data.wrapping_add(b.data) }))
    }

    pub fn interpret(mut self, cfa: *mut Word) -> Result<(), ()> {
        let mut words = unsafe {
            let len = *cfa.cast::<u32>() as usize;
            if len == 0 {
                return Ok(());
            }
            core::slice::from_raw_parts(cfa.add(1), len)
        }
        .iter();

        const LIT: *mut () = Fif::literal as *mut WordFunc<'static, 'static> as *mut ();
        while let Some(word) = words.next() {
            let ptr = unsafe { word.ptr };
            let builtin = Self::BUILTINS.iter().find_map(|(_name, func)| {
                let bif = (*func) as *mut ();
                if bif == ptr {
                    let a: WordFunc<'static, 'static> = *func;
                    let b: WordFunc<'_, '_> = unsafe { transmute(a) };
                    Some(b)
                } else {
                    None
                }
            });

            let fif2 = Fif {
                forth: self.forth,
                input: self.input,
            };
            if let Some(func) = builtin {
                func(fif2, null_mut())?;
            } else if LIT == ptr {
                let lit = words.next().ok_or(())?;
                let val = unsafe { lit.data };
                fif2.forth.data_stack.push(Word::data(val))?;
            } else {
                let (wf, cfa) = unsafe {
                    let de = NonNull::new_unchecked(ptr.cast::<DictionaryEntry>());
                    DictionaryEntry::get_run(de)
                };
                wf(fif2, cfa.as_ptr())?;
            }
        }

        Ok(())
    }

    pub fn literal(self, _cfa: *mut Word) -> Result<(), ()> {
        panic!();
        // TODO: Do I only use this as a sentinel?
        Err(())
    }

    pub fn colon(self, cfa: *mut Word) -> Result<(), ()> {
        let old_mode = core::mem::replace(&mut self.forth.mode, Mode::Compile);
        let name = self.input.next_word().ok_or(())?;
        let name = Name::new_from_bstr(Mode::Run, name.as_bytes());

        // TODO, I could check that there is at least a `;` here,
        // but that ignores any other errors. Let's plough ahead,
        // at the risk we "leak" dictionary memory in the case of
        // a bad compile. Later: we can figure out how to "unwind"
        // this and reclaim the allocated memory

        let word_base = self.forth.dict_alloc.bump::<DictionaryEntry>().ok_or(())?;

        unsafe {
            word_base.as_ptr().write(DictionaryEntry {
                name,
                // Don't link until we know we have a "good" entry!
                link: None,
                code_pointer: Fif::interpret,
                parameter_field: [],
            });
        }

        // Rather than having an "exit" word, I'll prepend the
        // cfa array with a length field (NOT including the length
        // itself).
        let len: &mut u32 = {
            let len_word = self.forth.dict_alloc.bump::<Word>().ok_or(())?;
            unsafe {
                len_word.as_ptr().write(Word::data(0));
                &mut (*len_word.as_ptr()).data
            }
        };

        let mut semicolon = false;

        while let Some(word) = self.input.next_word() {
            if word == ";" {
                semicolon = true;
                break;
            }
            match self.forth.lookup(word)? {
                Lookup::Builtin { func } => {
                    let fptr: *mut () = func as *mut ();
                    let wptr = self.forth.dict_alloc.bump::<Word>().ok_or(())?;
                    unsafe {
                        wptr.as_ptr().write(Word::ptr(fptr));
                    }
                    *len += 1;
                }
                Lookup::Dict { de } => {
                    let dptr: *mut () = de.as_ptr().cast();
                    let wptr = self.forth.dict_alloc.bump::<Word>().ok_or(())?;
                    unsafe {
                        wptr.as_ptr().write(Word::ptr(dptr));
                    }
                    *len += 1;
                }
                Lookup::Literal { val } => {
                    let fptr: *mut () = Fif::literal as *mut ();
                    let wptr = self.forth.dict_alloc.bump::<Word>().ok_or(())?;
                    unsafe {
                        wptr.as_ptr().write(Word::ptr(fptr));
                    }
                    *len += 1;

                    let wptr = self.forth.dict_alloc.bump::<Word>().ok_or(())?;
                    unsafe {
                        wptr.as_ptr().write(Word::data(val));
                    }
                    *len += 1;
                }
            }
        }

        if semicolon {
            // Link to run dict
            unsafe {
                (&mut *word_base.as_ptr()).link = self.forth.run_dict_tail.take();
            }
            self.forth.run_dict_tail = Some(word_base);
            self.forth.mode = old_mode;
            Ok(())
        } else {
            Err(())
        }
    }
}

pub enum Lookup<'a, 'b> {
    Builtin { func: WordFunc<'a, 'b> },
    Dict { de: NonNull<DictionaryEntry> },
    Literal { val: u32 },
}

type WordFunc<'a, 'b> = fn(Fif<'a, 'b>, *mut Word) -> Result<(), ()>;
impl Forth {
    pub unsafe fn new(stack_buf: (*mut Word, usize), dict_buf: (*mut u8, usize)) -> Self {
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

    fn find_in_dict<'a>(&self, word: &'a str) -> Option<NonNull<DictionaryEntry>> {
        let mut optr: Option<&NonNull<DictionaryEntry>> = self.run_dict_tail.as_ref();
        while let Some(ptr) = optr.take() {
            let de = unsafe { ptr.as_ref() };
            if de.name.as_str() == word {
                return Some(*ptr);
            }
            optr = de.link.as_ref();
        }
        None
    }

    fn find_builtin<'a, 'b>(word: &'b str) -> Option<WordFunc<'a, 'b>> {
        Fif::BUILTINS.iter().find_map(|(n, func)| {
            if *n == word {
                let func: WordFunc<'static, 'static> = *func;
                let func: WordFunc<'a, 'b> = unsafe { core::mem::transmute(func) };
                Some(func)
            } else {
                None
            }
        })
    }

    pub fn lookup<'a>(&self, word: &'a str) -> Result<Lookup<'_, 'a>, ()> {
        if let Some(func) = Self::find_builtin(word) {
            Ok(Lookup::Builtin { func })
        } else if let Some(entry) = self.find_in_dict(word) {
            Ok(Lookup::Dict { de: entry })
        } else if let Some(val) = Self::parse_num(word) {
            Ok(Lookup::Literal { val })
        } else {
            Err(())
        }
    }

    pub fn process_line<'a>(&mut self, line: &'a mut WordStrBuf) -> Result<(), ()> {
        while let Some(word) = line.next_word() {
            match self.lookup(word)? {
                Lookup::Builtin { func } => {
                    let before_compile = self.dict_alloc.cur;
                    let res = func(
                        Fif {
                            forth: self,
                            input: line,
                        },
                        null_mut(),
                    );

                    if func == Fif::colon {
                        if res.is_err() {
                            // Rewind the allocator to before the start of this compilation
                            println!("It's rewind time!");
                            println!(
                                "{:016X} => {:016X}",
                                self.dict_alloc.cur as usize,
                                before_compile as usize,
                            );
                            self.dict_alloc.cur = before_compile;
                        }
                        res
                    } else {
                        res
                    }
                },
                Lookup::Dict { de } => {
                    let (func, cfa) = unsafe { DictionaryEntry::get_run(de) };
                    func(
                        Fif {
                            forth: self,
                            input: line,
                        },
                        cfa.as_ptr(),
                    )
                }
                Lookup::Literal { val } => self.data_stack.push(Word::data(val)),
            }?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod test {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::UnsafeCell,
        mem::MaybeUninit,
        ptr::{addr_of_mut, NonNull},
    };

    use crate::{DictionaryBump, DictionaryEntry, Fif, Forth, Mode, Name, Stack, Word, WordStrBuf};

    #[test]
    fn forth() {
        let payload_stack: *mut Word = Box::leak(Box::new(UnsafeCell::new(MaybeUninit::<
            [Word; 256],
        >::uninit())))
        .get_mut()
        .as_mut_ptr()
        .cast();
        let input_buf: *mut u8 = Box::leak(Box::new(UnsafeCell::new(
            MaybeUninit::<[u8; 256]>::uninit(),
        )))
        .get_mut()
        .as_mut_ptr()
        .cast();
        let dict_buf: *mut u8 = Box::leak(Box::new(UnsafeCell::new(
            MaybeUninit::<[u8; 512]>::uninit(),
        )))
        .get_mut()
        .as_mut_ptr()
        .cast();

        let mut input = WordStrBuf::new(input_buf, 256);
        let mut forth = unsafe { Forth::new((payload_stack, 256), (dict_buf, 512)) };

        let lines = &[
            "2 3 add .",
            ": yay 2 3 add . ;",
            "yay yay yay",
            ": boop yay yay ;",
            "boop",
        ];

        for line in lines {
            println!("{}", line);
            print!(" => ");
            input.fill(line).unwrap();
            forth.process_line(&mut input).unwrap();
            println!("ok.");
        }

        input.fill(": derp boop yay").unwrap();
        assert!(forth.process_line(&mut input).is_err());

        input.fill(": doot yay yaay").unwrap();
        assert!(forth.process_line(&mut input).is_err());

        input.fill("boop yay").unwrap();
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
        assert_eq!(
            d.as_ptr()
                .align_offset(Layout::new::<DictionaryEntry>().align()),
            0
        );

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
