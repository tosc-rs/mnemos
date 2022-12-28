// For now...
#![allow(clippy::missing_safety_doc, clippy::result_unit_err)]

use core::fmt::Write;
use core::ptr::null_mut;
use core::{ptr::NonNull, str::FromStr};

pub mod dictionary;
pub mod input;
pub mod name;
pub mod output;
pub mod stack;
pub mod word;

use dictionary::BumpError;
use output::{OutputBuf, OutputError};
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
    Output(OutputError),
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
    WordToUsizeInvalid(i32),
    UsizeToWordInvalid(usize),
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

impl From<OutputError> for Error {
    fn from(oe: OutputError) -> Self {
        Error::Output(oe)
    }
}

/// `WordFunc` represents a function that can be used as part of a dictionary word.
///
/// It takes the current "full context" (e.g. `Fif`), as well as the CFA pointer
/// to the dictionary entry.
type WordFunc = fn(&mut Forth) -> Result<(), Error>;

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
    input: WordStrBuf,
    output: OutputBuf,

    // TODO: This will be for words that have compile time actions, I guess?
    _comp_dict_tail: Option<NonNull<DictionaryEntry>>,
}

impl Forth {
    pub unsafe fn new(
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        dict_buf: (*mut u8, usize),
        input: WordStrBuf,
        output: OutputBuf,
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
            input,
            output,
        };

        let mut last = None;

        for (name, func) in Forth::BUILTINS {
            let name = Name::new_from_bstr(Mode::Run, name.as_bytes());

            // Allocate and initialize the dictionary entry
            let dict_base = new.dict_alloc.bump::<DictionaryEntry>()?;
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

    pub fn process_line(
        &mut self,
        line: &mut WordStrBuf,
        out: &mut OutputBuf,
    ) -> Result<(), Error> {
        self.return_stack.push(Word::ptr(null_mut::<Word>()))?; // Fake CFA
        while let Some(word) = line.next_word() {
            match self.lookup(word)? {
                Lookup::Dict { de } => {
                    let (func, cfa) = unsafe { DictionaryEntry::get_run(de) };
                    self.return_stack.push(Word::data(0))?; // Fake offset
                    self.return_stack.push(Word::ptr(cfa.as_ptr()))?; // Calling CFA
                    let res = func(self);
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
        writeln!(out, "ok.").map_err(|_| OutputError::FormattingErr)?;
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
impl Forth {
    const BUILTINS: &'static [(&'static str, WordFunc)] = &[
        ("+", Forth::add),
        ("dup", Forth::dup),
        (".", Forth::pop_print),
        (":", Forth::colon),
        ("(literal)", Forth::literal),
        ("d>r", Forth::data_to_return_stack),
        ("r>d", Forth::return_to_data_stack),
    ];

    pub fn dup(&mut self) -> Result<(), Error> {
        let val = self.data_stack.peek().ok_or(StackError::StackEmpty)?;
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn return_to_data_stack(&mut self) -> Result<(), Error> {
        let val = self
            .return_stack
            .pop()
            .ok_or(StackError::StackEmpty)?;
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn data_to_return_stack(&mut self) -> Result<(), Error> {
        let val = self.data_stack.pop().ok_or(StackError::StackEmpty)?;
        self.return_stack.push(val)?;
        Ok(())
    }

    pub fn pop_print(&mut self) -> Result<(), Error> {
        let a = self.data_stack.pop().ok_or(StackError::StackEmpty)?;
        write!(&mut self.output, "{} ", unsafe { a.data })
            .map_err(|_| OutputError::FormattingErr)?;
        Ok(())
    }

    pub fn add(&mut self) -> Result<(), Error> {
        let a = self.data_stack.pop().ok_or(StackError::StackEmpty)?;
        let b = self.data_stack.pop().ok_or(StackError::StackEmpty)?;
        self
            .data_stack
            .push(Word::data(unsafe { a.data.wrapping_add(b.data) }))?;
        Ok(())
    }

    pub fn colon(&mut self) -> Result<(), Error> {
        let name = self
            .input
            .next_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let old_mode = core::mem::replace(&mut self.mode, Mode::Compile);
        let name = Name::new_from_bstr(Mode::Run, name.as_bytes());
        let literal_dict = self
            .find_in_dict("(literal)")
            .ok_or(Error::WordNotInDict)?;

        // Allocate and initialize the dictionary entry
        //
        // TODO: Using `bump_write` here instead of just `bump` causes Miri to
        // get angry with a stacked borrows violation later when we attempt
        // to interpret a built word.
        let mut dict_base = self.dict_alloc.bump::<DictionaryEntry>()?;
        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
                name,
                // Don't link until we know we have a "good" entry!
                link: None,
                code_pointer: Forth::interpret,
                parameter_field: [],
            });
        }

        // Rather than having an "exit" word, I'll prepend the
        // cfa array with a length field (NOT including the length
        // itself).
        let len: &mut i32 = {
            let len_word = self.dict_alloc.bump::<Word>()?;
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
            match self.lookup(word)? {
                Lookup::Dict { de } => {
                    // Dictionary items are put into the CFA array directly as
                    // a pointer to the dictionary entry
                    let dptr: *mut () = de.as_ptr().cast();
                    self.dict_alloc.bump_write(Word::ptr(dptr))?;
                    *len += 1;
                }
                Lookup::Literal { val } => {
                    // Literals are added to the CFA as two items:
                    //
                    // 1. The address of the `literal()` dictionary item
                    // 2. The value of the literal, as a data word
                    self
                        .dict_alloc
                        .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                    self.dict_alloc.bump_write(Word::data(val))?;
                    *len += 2;
                }
            }
        }

        // Did we successfully get to the end, marked by a semicolon?
        if semicolon {
            // Link to run dict
            unsafe {
                dict_base.as_mut().link = self.run_dict_tail.take();
            }
            self.run_dict_tail = Some(dict_base);
            self.mode = old_mode;
            Ok(())
        } else {
            Err(Error::ColonCompileMissingSemicolon)
        }
    }

    /// `(literal)` is used mid-interpret to put the NEXT word of the parent's
    /// CFA array into the stack as a value.
    pub fn literal(&mut self) -> Result<(), Error> {
        // Current stack SHOULD be:
        // 0: OUR CFA (d/c)
        // 1: Our parent's CFA offset
        // 2: Out parent's CFA
        let parent_cfa = self
            .return_stack
            .peek_back_n(2)
            .ok_or(Error::ReturnStackMissingParentCFA)?;
        let parent_off: usize = self
            .return_stack
            .peek_back_n(1)
            .ok_or(Error::ReturnStackMissingParentCFAIdx)?
            .try_into()?;

        // Our parent is calling *US*, so the literal is the *NEXT* word.
        let lit_offset = parent_off + 1;

        unsafe {
            // Turn the parent's CFA into a slice
            let putter = parent_cfa.ptr.cast::<Word>();
            let sli = cfa_to_slice(putter);

            // Then try to get the value at the expected location
            let val = *sli
                .get(lit_offset)
                .ok_or(Error::CFAIdxInInvalid(lit_offset))?;
            // Put the value on the data stack
            self.data_stack.push(val)?;
            // Move the "program counter" to the literal, so our parent "thinks"
            // they just processed the literal
            self
                .return_stack
                .overwrite_back_n(1, lit_offset.try_into()?)?;
        }
        Ok(())
    }

    /// Interpret is the run-time target of the `:` (colon) word.
    ///
    /// It is NOT considered a "builtin", as it DOES take the cfa, where
    /// other builtins do not.
    pub fn interpret(&mut self) -> Result<(), Error> {
        // Colon compiles into a list of words, where the first word
        // is a `u32` of the `len` number of words.
        let words = unsafe {
            let cfa = self
                .return_stack
                .peek()
                .ok_or(Error::ReturnStackMissingCFA)?
                .ptr
                .cast::<Word>();

            cfa_to_slice(cfa)
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
            let in_dict = self.dict_alloc.contains(ptr);

            if in_dict {
                // If the word points to somewhere in the dictionary, then treat
                // it as if it is a dictionary entry
                let (wf, cfa) = unsafe {
                    let de = NonNull::new_unchecked(ptr.cast::<DictionaryEntry>());
                    DictionaryEntry::get_run(de)
                };

                // We then call the dictionary entry's function with the cfa addr.
                let idx_word = idx.try_into()?;
                self.return_stack.push(idx_word)?; // Our "index"
                self.return_stack.push(Word::ptr(cfa.as_ptr()))?; // Callee CFA
                wf(self)?;
                self
                    .return_stack
                    .pop()
                    .ok_or(Error::ReturnStackMissingCFA)?;
                let oidx_i32 = self
                    .return_stack
                    .pop()
                    .ok_or(Error::ReturnStackMissingCFAIdx)?;
                idx = oidx_i32.try_into()?;
            } else {
                return Err(Error::CFANotInDict(*word));
            }
            idx += 1;
            // TODO: If I want A4-style pausing here, I'd probably want to also
            // push dictionary locations to the stack (under the CFA), which
            // would allow for halting and resuming. Yield after loading "next",
            // right before executing the function itself. This would also allow
            // for cursed control flow
        }
        Ok(())
    }
}

unsafe fn cfa_to_slice<'a>(ptr: *mut Word) -> &'a [Word] {
    // First is length
    let len = (*ptr).data as usize;
    core::slice::from_raw_parts(ptr.add(1), len)
}

pub enum Lookup {
    Dict { de: NonNull<DictionaryEntry> },
    Literal { val: i32 },
}

#[cfg(test)]
pub mod test {
    use std::{cell::UnsafeCell, mem::MaybeUninit};

    use crate::{output::OutputBuf, Forth, Word, WordStrBuf};

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
        let output_buf: LeakBox<u8, 256> = LeakBox::new();
        let dict_buf: LeakBox<u8, 4096> = LeakBox::new();

        let mut input = WordStrBuf::new(input_buf.ptr(), input_buf.len());
        let mut output = OutputBuf::new(output_buf.ptr(), output_buf.len());
        let mut forth = unsafe {
            Forth::new(
                (payload_dstack.ptr(), payload_dstack.len()),
                (payload_rstack.ptr(), payload_rstack.len()),
                (dict_buf.ptr(), dict_buf.len()),
            )
            .unwrap()
        };

        let lines = &[
            ("2 3 + .", "5 ok.\n"),
            (": yay 2 3 + . ;", "ok.\n"),
            ("yay yay yay", "5 5 5 ok.\n"),
            (": boop yay yay ;", "ok.\n"),
            ("boop", "5 5 ok.\n"),
        ];

        for (line, out) in lines {
            println!("{}", line);
            input.fill(line).unwrap();
            forth.process_line(&mut input, &mut output).unwrap();
            assert_eq!(output.as_str(), *out);
            output.clear();
        }

        input.fill(": derp boop yay").unwrap();
        assert!(forth.process_line(&mut input, &mut output).is_err());

        input.fill(": doot yay yaay").unwrap();
        assert!(forth.process_line(&mut input, &mut output).is_err());

        output.clear();
        input.fill("boop yay").unwrap();
        forth.process_line(&mut input, &mut output).unwrap();
        assert_eq!(output.as_str(), "5 5 5 ok.\n");

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
