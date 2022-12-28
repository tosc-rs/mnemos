// For now...
#![allow(clippy::missing_safety_doc, clippy::result_unit_err)]

use core::{ptr::NonNull, str::FromStr};
use std::ptr::null_mut;

pub mod dictionary;
pub mod input;
pub mod name;
pub mod stack;
pub mod word;

use dictionary::BumpError;
use stack::StackError;

use crate::{
    dictionary::{DictionaryBump, DictionaryEntry},
    input::WordStrBuf,
    name::{Mode, Name},
    stack::Stack,
    word::Word,
};

#[derive(Debug, PartialEq)]
pub enum Error {
    Stack(StackError),
    Bump(BumpError),
    ReturnStackMissingCFA,
    ReturnStackMissingCFAIdx,
    ReturnStackMissingParentCFA,
    ReturnStackMissingParentCFAIdx,
    CFANotInDict(Word),
    CFAIdxOutInvalid(Word),
    WordNotInDict,
    CFAIdxInInvalid(usize),
    ColonCompileMissingName,
    ColonCompileMissingSemicolon,
    LookupFailed,
    ShouldBeUnreachable,
}

impl From<StackError> for Error {
    fn from(se: StackError) -> Self {
        Error::Stack(se)
    }
}

impl From<BumpError> for Error {
    fn from(be: BumpError) -> Self {
        Error::Bump(be)
    }
}

/// `WordFunc` represents a function that can be used as part of a dictionary word.
///
/// It takes the current "full context" (e.g. `Fif`), as well as the CFA pointer
/// to the dictionary entry.
type WordFunc<'a, 'b> = fn(Fif<'a, 'b>) -> Result<(), Error>;

/// Forth is the "context" of the VM/interpreter.
///
/// It does NOT include the input/output buffers, or any components that
/// directly rely on those buffers. This Forth context is composed with
/// the I/O buffers to create the `Fif` type. This is done for lifetime
/// reasons.
pub struct Forth {
    mode: Mode,
    data_stack: Stack,
    return_stack: Stack,
    dict_alloc: DictionaryBump,
    run_dict_tail: Option<NonNull<DictionaryEntry>>,

    // TODO: This will be for words that have compile time actions, I guess?
    _comp_dict_tail: Option<NonNull<DictionaryEntry>>,
}

impl Forth {
    pub unsafe fn new(
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        dict_buf: (*mut u8, usize),
    ) -> Result<Self, Error> {
        let data_stack = Stack::new(dstack_buf.0, dstack_buf.1);
        let return_stack = Stack::new(rstack_buf.0, rstack_buf.1);
        let dict_alloc = DictionaryBump::new(dict_buf.0, dict_buf.1);
        let mut new = Self {
            mode: Mode::Run,
            data_stack,
            return_stack,
            dict_alloc,
            run_dict_tail: None,
            _comp_dict_tail: None,
        };

        let mut last = None;

        for (name, func) in Fif::BUILTINS {
            let name = Name::new_from_bstr(Mode::Run, name.as_bytes());

            // Allocate and initialize the dictionary entry
            let dict_base = new.dict_alloc.bump::<DictionaryEntry>()?;
            println!(
                "INIT CMP: '{}' - {:016X}",
                name.as_str(),
                dict_base.as_ptr() as usize
            );
            unsafe {
                dict_base.as_ptr().write(DictionaryEntry {
                    name,
                    link: last.take(),
                    code_pointer: *func,
                    parameter_field: [],
                });
            }
            last = Some(dict_base);
        }

        new.run_dict_tail = last;
        Ok(new)
    }

    fn parse_num(word: &str) -> Option<i32> {
        i32::from_str(word).ok()
    }

    fn find_in_dict(&self, word: &str) -> Option<NonNull<DictionaryEntry>> {
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

    pub fn lookup(&self, word: &str) -> Result<Lookup, Error> {
        if let Some(entry) = self.find_in_dict(word) {
            Ok(Lookup::Dict { de: entry })
        } else if let Some(val) = Self::parse_num(word) {
            Ok(Lookup::Literal { val })
        } else {
            Err(Error::LookupFailed)
        }
    }

    pub fn process_line(&mut self, line: &mut WordStrBuf) -> Result<(), Error> {
        self.return_stack.push(Word::ptr(null_mut::<Word>()))?; // Fake CFA
        while let Some(word) = line.next_word() {
            match self.lookup(word)? {
                Lookup::Dict { de } => {
                    println!("PLLU - {}", word);
                    let (func, cfa) = unsafe { DictionaryEntry::get_run(de) };
                    self.return_stack.push(Word::data(0))?; // Fake offset
                    self.return_stack.push(Word::ptr(cfa.as_ptr()))?; // Calling CFA
                    let res = func(Fif {
                        forth: self,
                        input: line,
                    });
                    self.return_stack
                        .pop()
                        .ok_or(Error::ReturnStackMissingCFA)?;
                    self.return_stack
                        .pop()
                        .ok_or(Error::ReturnStackMissingParentCFAIdx)?;
                    res?;
                }
                Lookup::Literal { val } => {
                    self.data_stack.push(Word::data(val))?;
                }
            }
        }
        self.return_stack
            .pop()
            .ok_or(Error::ReturnStackMissingParentCFA)?;
        Ok(())
    }
}

/// `Fif` is an ephemeral container that holds both the Forth interpreter/VM
/// as well as the I/O buffers.
///
/// This was originally done to keep the lifetimes separate, so we could
/// mutate the I/O buffer (mostly popping values) while operating on the
/// forth VM. It may be possible to move `Fif`'s functionality back into the
/// `Forth` struct at a later point.
pub struct Fif<'a, 'b> {
    forth: &'a mut Forth,
    input: &'b mut WordStrBuf,
}

impl<'a, 'b> Fif<'a, 'b> {
    const BUILTINS: &'static [(&'static str, WordFunc<'static, 'static>)] = &[
        ("add", Fif::add),
        (".", Fif::pop_print),
        (":", Fif::colon),
        ("(literal)", Fif::literal),
    ];

    pub fn pop_print(self) -> Result<(), Error> {
        let _a = self.forth.data_stack.pop().ok_or(StackError::StackEmpty)?;
        #[cfg(test)]
        print!("{} ", unsafe { _a.data });
        Ok(())
    }

    pub fn add(self) -> Result<(), Error> {
        let a = self.forth.data_stack.pop().ok_or(StackError::StackEmpty)?;
        let b = self.forth.data_stack.pop().ok_or(StackError::StackEmpty)?;
        self.forth
            .data_stack
            .push(Word::data(unsafe { a.data.wrapping_add(b.data) }))?;
        Ok(())
    }

    pub fn colon(self) -> Result<(), Error> {
        let name = self
            .input
            .next_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let old_mode = core::mem::replace(&mut self.forth.mode, Mode::Compile);
        let name = Name::new_from_bstr(Mode::Run, name.as_bytes());
        let literal_dict = self
            .forth
            .find_in_dict("(literal)")
            .ok_or(Error::WordNotInDict)?;

        // Allocate and initialize the dictionary entry
        //
        // TODO: Using `bump_write` here instead of just `bump` causes Miri to
        // get angry with a stacked borrows violation later when we attempt
        // to interpret a built word.
        let mut dict_base = self.forth.dict_alloc.bump::<DictionaryEntry>()?;
        println!(
            "RUNT CMP: '{}' - {:016X}",
            name.as_str(),
            dict_base.as_ptr() as usize
        );
        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
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
        let len: &mut i32 = {
            let len_word = self.forth.dict_alloc.bump::<Word>()?;
            unsafe {
                len_word.as_ptr().write(Word::data(0));
                &mut (*len_word.as_ptr()).data
            }
        };

        // Begin compiling until we hit the end of the line or a semicolon.
        let mut semicolon = false;
        while let Some(word) = self.input.next_word() {
            if word == ";" {
                semicolon = true;
                break;
            }
            match self.forth.lookup(word)? {
                Lookup::Dict { de } => {
                    // Dictionary items are put into the CFA array directly as
                    // a pointer to the dictionary entry
                    let dptr: *mut () = de.as_ptr().cast();
                    self.forth.dict_alloc.bump_write(Word::ptr(dptr))?;
                    *len += 1;
                }
                Lookup::Literal { val } => {
                    // Literals are added to the CFA as two items:
                    //
                    // 1. The address of the `literal()` dictionary item
                    // 2. The value of the literal, as a data word
                    self.forth
                        .dict_alloc
                        .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                    self.forth.dict_alloc.bump_write(Word::data(val))?;
                    *len += 2;
                }
            }
        }

        // Did we successfully get to the end, marked by a semicolon?
        if semicolon {
            // Link to run dict
            unsafe {
                dict_base.as_mut().link = self.forth.run_dict_tail.take();
            }
            self.forth.run_dict_tail = Some(dict_base);
            self.forth.mode = old_mode;
            Ok(())
        } else {
            Err(Error::ColonCompileMissingSemicolon)
        }
    }

    /// Literal is generally only used as a sentinel value in compiled
    /// words to note that the next item in the CFA is a `u32`.
    ///
    /// It generally shouldn't ever be called.
    pub fn literal(self) -> Result<(), Error> {
        // Current stack SHOULD be:
        // 0: OUR CFA (d/c)
        // 1: Our parent's CFA offset
        // 2: Out parent's CFA
        let parent_cfa = self
            .forth
            .return_stack
            .peek_back_n(2)
            .ok_or(Error::ReturnStackMissingParentCFA)?;
        let parent_off = self
            .forth
            .return_stack
            .peek_back_n(1)
            .ok_or(Error::ReturnStackMissingParentCFAIdx)?;
        unsafe {
            let putter = parent_cfa.ptr.cast::<Word>();
            let val = putter.offset(parent_off.data as isize).read();
            self.forth.data_stack.push(val)?;
            // Increment our parent's offset by one to skip the literal
            self.forth
                .return_stack
                .overwrite_back_n(1, Word::data(parent_off.data + 1))?;
        }
        Ok(())
    }

    /// Interpret is the run-time target of the `:` (colon) word.
    ///
    /// It is NOT considered a "builtin", as it DOES take the cfa, where
    /// other builtins do not.
    pub fn interpret(self) -> Result<(), Error> {
        // The "literal" built-in word is used as a sentinel. See below.
        let cfa = unsafe {
            self.forth
                .return_stack
                .peek()
                .ok_or(Error::ReturnStackMissingCFA)?
                .ptr
                .cast::<Word>()
        };

        // Colon compiles into a list of words, where the first word
        // is a `u32` of the `len` number of words.
        let words = unsafe {
            let len = *cfa.cast::<u32>() as usize;
            if len == 0 {
                return Ok(());
            }
            // Skip the "len" field, which is the first word
            // of the cfa.
            core::slice::from_raw_parts(cfa.add(1), len)
        };

        let mut idx = 0usize;
        // For the remaining words, we do a while-let loop instead of
        // a for-loop, as some words (e.g. literals) require advancing
        // to the next word.
        while let Some(word) = words.get(idx) {
            // We can safely assume that all items in the list are pointers,
            // EXCEPT for literals, but those are handled manually below.
            let ptr = unsafe { word.ptr };

            // Is the given word pointing at somewhere in the range of
            // the dictionary allocator?
            let in_dict = self.forth.dict_alloc.contains(ptr);

            // We need to re-borrow our fields to make another `Fif`, which
            // is basically just `self` but gets consumed by the function we
            // call.
            let fif2 = Fif {
                forth: self.forth,
                input: self.input,
            };

            if in_dict {
                // If the word points to somewhere in the dictionary, then treat
                // it as if it is a dictionary entry
                let (wf, cfa) = unsafe {
                    let de = NonNull::new_unchecked(ptr.cast::<DictionaryEntry>());
                    DictionaryEntry::get_run(de)
                };

                // We then call the dictionary entry's function with the cfa addr.
                let idx_i32 = i32::try_from(idx).map_err(|_| Error::CFAIdxInInvalid(idx))?;
                fif2.forth.return_stack.push(Word::data(idx_i32))?; // Our "index"
                fif2.forth.return_stack.push(Word::ptr(cfa.as_ptr()))?; // Callee CFA
                wf(fif2)?;
                self.forth
                    .return_stack
                    .pop()
                    .ok_or(Error::ReturnStackMissingCFA)?;
                let oidx_i32 = self
                    .forth
                    .return_stack
                    .pop()
                    .ok_or(Error::ReturnStackMissingCFAIdx)?;
                idx = usize::try_from(unsafe { oidx_i32.data })
                    .map_err(|_| Error::CFAIdxOutInvalid(oidx_i32))?;
            } else {
                println!("UH OH: {:016X}", ptr as usize);
                return Err(Error::CFANotInDict(*word));
            }
            idx += 1;
        }

        Ok(())
    }
}

pub enum Lookup {
    Dict { de: NonNull<DictionaryEntry> },
    Literal { val: i32 },
}

#[cfg(test)]
pub mod test {
    use std::{cell::UnsafeCell, mem::MaybeUninit};

    use crate::{Forth, Word, WordStrBuf};

    // Helper type that will un-leak the buffer once it is dropped.
    pub(crate) struct LeakBox<T, const N: usize> {
        ptr: *mut UnsafeCell<MaybeUninit<[T; N]>>,
    }

    impl<T, const N: usize> LeakBox<T, N> {
        pub(crate) fn new() -> Self {
            Self {
                ptr: Box::leak(Box::new(UnsafeCell::new(MaybeUninit::uninit()))),
            }
        }

        pub(crate) fn ptr(&self) -> *mut T {
            self.ptr.cast()
        }

        pub(crate) fn len(&self) -> usize {
            N
        }
    }

    impl<T, const N: usize> Drop for LeakBox<T, N> {
        fn drop(&mut self) {
            unsafe {
                let _ = Box::from_raw(self.ptr);
            }
        }
    }

    #[test]
    fn forth() {
        let payload_dstack: LeakBox<Word, 256> = LeakBox::new();
        let payload_rstack: LeakBox<Word, 256> = LeakBox::new();
        let input_buf: LeakBox<u8, 256> = LeakBox::new();
        let dict_buf: LeakBox<u8, 512> = LeakBox::new();

        let mut input = WordStrBuf::new(input_buf.ptr(), input_buf.len());
        let mut forth = unsafe {
            Forth::new(
                (payload_dstack.ptr(), payload_dstack.len()),
                (payload_rstack.ptr(), payload_rstack.len()),
                (dict_buf.ptr(), dict_buf.len()),
            )
            .unwrap()
        };

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

        // Uncomment if you want to check how much of the dictionary
        // was used during a test run.
        //
        // assert_eq!(176, forth.dict_alloc.used());

        // Uncomment this if you want to see the output of the
        // forth run. TODO: Remove this once we implement the
        // output buffer.
        //
        // panic!();
    }
}
