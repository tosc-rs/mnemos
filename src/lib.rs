// For now...
#![allow(clippy::missing_safety_doc, clippy::result_unit_err)]

pub mod dictionary;
pub mod fastr;
pub mod input;
pub mod output;
pub mod stack;
pub mod word;

use core::{fmt::Write, ops::Deref, ptr::NonNull, str::FromStr};

use crate::{
    dictionary::{BumpError, DictionaryBump, DictionaryEntry},
    fastr::{FaStr, TmpFaStr},
    input::WordStrBuf,
    output::{OutputBuf, OutputError},
    stack::{Stack, StackError},
    word::Word,
};

#[derive(Debug)]
pub enum Mode {
    Run,
    Compile,
}

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
    ElseBeforeIf,
    ThenBeforeIf,
    IfWithoutThen,
    DuplicateElse,
    IfElseWithoutThen,
    CallStackCorrupted,
    InterpretingCompileOnlyWord,
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

#[derive(Copy, Clone)]
pub struct Context {
    cfa: *mut Word,
    idx: usize,
}

impl Context {
    fn get_next_val(&self) -> Result<i32, Error> {
        unsafe {
            let sli = cfa_to_slice(self.cfa);
            let req = self.idx + 1;
            Ok(sli.get(req).ok_or(Error::CFAIdxInInvalid(req))?.data)
        }
    }

    fn offset(&mut self, offset: i32) -> Result<(), Error> {
        // TODO JAMES THIS IS BAD
        if offset.is_positive() {
            self.idx += offset as usize;
        } else {
            self.idx -= offset.unsigned_abs() as usize;
        }
        Ok(())
    }

    fn cfa_arr(&self) -> &[Word] {
        unsafe { cfa_to_slice(self.cfa) }
    }

    fn get_word(&self) -> Option<&Word> {
        let arr = self.cfa_arr();
        arr.get(self.idx)
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
    data_stack: Stack<Word>,
    return_stack: Stack<Word>,
    call_stack: Stack<Context>,
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
        cstack_buf: (*mut Context, usize),
        dict_buf: (*mut u8, usize),
        input: WordStrBuf,
        output: OutputBuf,
    ) -> Result<Self, Error> {
        let data_stack = Stack::new(dstack_buf.0, dstack_buf.1);
        let return_stack = Stack::new(rstack_buf.0, rstack_buf.1);
        let call_stack = Stack::new(cstack_buf.0, cstack_buf.1);
        let dict_alloc = DictionaryBump::new(dict_buf.0, dict_buf.1);
        let mut new = Self {
            mode: Mode::Run,
            data_stack,
            return_stack,
            call_stack,
            dict_alloc,
            run_dict_tail: None,
            _comp_dict_tail: None,
            input,
            output,
        };

        let mut last = None;

        for (name, func) in Forth::BUILTINS {
            let name = unsafe { FaStr::new(name.as_ptr(), name.len()) };

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
        let fastr = TmpFaStr::new_from(word);
        while let Some(ptr) = optr.take() {
            let de = unsafe { ptr.as_ref() };
            if &de.name == fastr.deref() {
                return Some(*ptr);
            }
            optr = de.link.as_ref();
        }
        None
    }

    pub fn lookup(&self, word: &str) -> Result<Lookup, Error> {
        match word {
            ";" => Ok(Lookup::Semicolon),
            "if" => Ok(Lookup::If),
            "else" => Ok(Lookup::Else),
            "then" => Ok(Lookup::Then),
            _ => {
                if let Some(entry) = self.find_in_dict(word) {
                    Ok(Lookup::Dict { de: entry })
                } else if let Some(val) = Self::parse_num(word) {
                    Ok(Lookup::Literal { val })
                } else {
                    Err(Error::LookupFailed)
                }
            }
        }
    }

    pub fn process_line(&mut self) -> Result<(), Error> {
        loop {
            self.input.advance();
            let word = match self.input.cur_word() {
                Some(w) => w,
                None => break,
            };

            match self.lookup(word)? {
                Lookup::Dict { de } => {
                    let (func, cfa) = unsafe { DictionaryEntry::get_run(de) };
                    self.call_stack.push(Context {
                        cfa: cfa.as_ptr(),
                        idx: 0,
                    })?;
                    let res = func(self);
                    self.call_stack.pop().ok_or(Error::CallStackCorrupted)?;
                    res?;
                }
                Lookup::Literal { val } => {
                    self.data_stack.push(Word::data(val))?;
                }
                Lookup::Semicolon => return Err(Error::InterpretingCompileOnlyWord),
                Lookup::If => return Err(Error::InterpretingCompileOnlyWord),
                Lookup::Else => return Err(Error::InterpretingCompileOnlyWord),
                Lookup::Then => return Err(Error::InterpretingCompileOnlyWord),
            }
        }
        writeln!(&mut self.output, "ok.").map_err(|_| OutputError::FormattingErr)?;
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
        ("(jump-zero)", Forth::jump_if_zero),
        ("(jmp)", Forth::jump),
        ("emit", Forth::emit),
    ];

    pub fn emit(&mut self) -> Result<(), Error> {
        let val = self.data_stack.pop().ok_or(StackError::StackEmpty)?;
        let val = unsafe { val.data };
        self.output.push_bstr(&[val as u8])?;
        Ok(())
    }

    pub fn jump_if_zero(&mut self) -> Result<(), Error> {
        let do_jmp = unsafe {
            let val = self.data_stack.pop().ok_or(StackError::StackEmpty)?;
            val.data == 0
        };
        if do_jmp {
            self.jump()
        } else {
            let parent = self.call_stack.peek_back_n_mut(1).unwrap();
            parent.offset(1).unwrap();
            Ok(())
        }
    }

    pub fn jump(&mut self) -> Result<(), Error> {
        let mut parent = self.call_stack.peek_back_n(1).unwrap();
        let offset = parent.get_next_val()?;
        parent.offset(offset)?;
        self.call_stack.overwrite_back_n(1, parent)?;
        Ok(())
    }

    pub fn dup(&mut self) -> Result<(), Error> {
        let val = self.data_stack.peek().ok_or(StackError::StackEmpty)?;
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn return_to_data_stack(&mut self) -> Result<(), Error> {
        let val = self.return_stack.pop().ok_or(StackError::StackEmpty)?;
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
        self.data_stack
            .push(Word::data(unsafe { a.data.wrapping_add(b.data) }))?;
        Ok(())
    }

    pub fn colon(&mut self) -> Result<(), Error> {
        self.input.advance();
        let name = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let old_mode = core::mem::replace(&mut self.mode, Mode::Compile);
        let name = self.dict_alloc.bump_str(name)?;

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
        while self.munch_one(len)? != 0 {}

        // Did we successfully get to the end, marked by a semicolon?
        if self.input.cur_word() == Some(";") {
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

    fn munch_if(&mut self, len: &mut i32) -> Result<i32, Error> {
        let start = *len;

        // Write a conditional jump, followed by space for a literal
        let literal_cj = self
            .find_in_dict("(jump-zero)")
            .ok_or(Error::WordNotInDict)?;
        self.dict_alloc.bump_write(Word::ptr(literal_cj.as_ptr()))?;
        let cj_offset: &mut i32 = {
            let cj_offset_word = self.dict_alloc.bump::<Word>()?;
            unsafe {
                cj_offset_word.as_ptr().write(Word::data(0));
                &mut (*cj_offset_word.as_ptr()).data
            }
        };

        // Increment the length for the number so far.
        *len += 2;

        let mut else_then = false;
        let if_start = *len;
        // Now work until we hit an else or then statement.
        loop {
            match self.munch_one(len) {
                // We hit the end of stream before an else/then.
                Ok(0) => return Err(Error::IfWithoutThen),
                // We compiled some stuff, keep going...
                Ok(_) => {}
                Err(Error::ElseBeforeIf) => {
                    else_then = true;
                    break;
                }
                Err(Error::ThenBeforeIf) => break,
                Err(e) => return Err(e),
            }
        }

        let delta = *len - if_start;
        if !else_then {
            // we got a "then"
            //
            // Jump offset is words placed + 1 for the jump-zero literal
            *cj_offset = delta + 1;
            return Ok(*len - start);
        }
        // We got an "else", keep going for "then"
        //
        // Jump offset is words placed + 1 (cj lit) + 2 (else cj + lit)
        *cj_offset = delta + 3;

        // Write a conditional jump, followed by space for a literal
        let literal_jmp = self.find_in_dict("(jmp)").ok_or(Error::WordNotInDict)?;
        self.dict_alloc
            .bump_write(Word::ptr(literal_jmp.as_ptr()))?;
        let jmp_offset: &mut i32 = {
            let jmp_offset_word = self.dict_alloc.bump::<Word>()?;
            unsafe {
                jmp_offset_word.as_ptr().write(Word::data(0));
                &mut (*jmp_offset_word.as_ptr()).data
            }
        };
        *len += 2;

        let else_start = *len;
        // Now work until we hit a then statement.
        loop {
            match self.munch_one(len) {
                // We hit the end of stream before a then.
                Ok(0) => return Err(Error::IfElseWithoutThen),
                // We compiled some stuff, keep going...
                Ok(_) => {}
                Err(Error::ElseBeforeIf) => return Err(Error::DuplicateElse),
                Err(Error::ThenBeforeIf) => break,
                Err(e) => return Err(e),
            }
        }

        let delta = *len - else_start;
        // Jump offset is words placed + 1 (jmp lit)
        *jmp_offset = delta + 1;

        Ok(*len - start)
    }

    fn munch_one(&mut self, len: &mut i32) -> Result<i32, Error> {
        let start = *len;
        self.input.advance();
        let word = match self.input.cur_word() {
            Some(w) => w,
            None => return Ok(0),
        };

        match self.lookup(word)? {
            Lookup::If => return self.munch_if(len),
            Lookup::Else => return Err(Error::ElseBeforeIf),
            Lookup::Then => return Err(Error::ThenBeforeIf),
            Lookup::Semicolon => return Ok(0),
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
                let literal_dict = self.find_in_dict("(literal)").ok_or(Error::WordNotInDict)?;
                self.dict_alloc
                    .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                self.dict_alloc.bump_write(Word::data(val))?;
                *len += 2;
            }
        }
        Ok(*len - start)
    }

    /// `(literal)` is used mid-interpret to put the NEXT word of the parent's
    /// CFA array into the stack as a value.
    pub fn literal(&mut self) -> Result<(), Error> {
        // Current stack SHOULD be:
        // 0: OUR CFA (d/c)
        // 1: Our parent's CFA offset
        // 2: Out parent's CFA
        let parent = self.call_stack.peek_back_n_mut(1).unwrap();
        let literal = parent.get_next_val()?;
        parent.offset(1)?;
        self.data_stack.push(Word::data(literal))?;
        Ok(())
    }

    /// Interpret is the run-time target of the `:` (colon) word.
    ///
    /// It is NOT considered a "builtin", as it DOES take the cfa, where
    /// other builtins do not.
    pub fn interpret(&mut self) -> Result<(), Error> {
        // Colon compiles into a list of words, where the first word
        // is a `u32` of the `len` number of words.
        //
        // NOTE: we DON'T use `Stack::peek_back_n_mut` because the callee
        // could pop off our item, which would lead to UB.
        let mut me = self.call_stack.peek().unwrap();

        // For the remaining words, we do a while-let loop instead of
        // a for-loop, as some words (e.g. literals) require advancing
        // to the next word.
        while let Some(word) = me.get_word() {
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

                self.call_stack.overwrite_back_n(0, me)?;
                self.call_stack.push(Context {
                    cfa: cfa.as_ptr(),
                    idx: 0,
                })?;
                let result = wf(self);
                self.call_stack.pop().unwrap();
                result?;
                me = self.call_stack.peek().unwrap();
            } else {
                return Err(Error::CFANotInDict(*word));
            }
            me.offset(1).unwrap();
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
    Semicolon,
    If,
    Else,
    Then,
}

#[cfg(test)]
pub mod test {
    use std::{cell::UnsafeCell, mem::MaybeUninit};

    use crate::{output::OutputBuf, Context, Forth, Word, WordStrBuf};

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
        let payload_cstack: LeakBox<Context, 256> = LeakBox::new();
        let input_buf: LeakBox<u8, 256> = LeakBox::new();
        let output_buf: LeakBox<u8, 256> = LeakBox::new();
        let dict_buf: LeakBox<u8, 4096> = LeakBox::new();

        let input = WordStrBuf::new(input_buf.ptr(), input_buf.len());
        let output = OutputBuf::new(output_buf.ptr(), output_buf.len());
        let mut forth = unsafe {
            Forth::new(
                (payload_dstack.ptr(), payload_dstack.len()),
                (payload_rstack.ptr(), payload_rstack.len()),
                (payload_cstack.ptr(), payload_cstack.len()),
                (dict_buf.ptr(), dict_buf.len()),
                input,
                output,
            )
            .unwrap()
        };

        let lines = &[
            ("2 3 + .", "5 ok.\n"),
            (": yay 2 3 + . ;", "ok.\n"),
            ("yay yay yay", "5 5 5 ok.\n"),
            (": boop yay yay ;", "ok.\n"),
            ("boop", "5 5 ok.\n"),
            (": err if boop boop boop else yay yay then ;", "ok.\n"),
            (": erf if boop boop boop then yay yay ;", "ok.\n"),
            ("0 err", "5 5 ok.\n"),
            ("1 err", "5 5 5 5 5 5 ok.\n"),
            ("0 erf", "5 5 ok.\n"),
            ("1 erf", "5 5 5 5 5 5 5 5 ok.\n"),
            (": one 1 . ;", "ok.\n"),
            (": two 2 . ;", "ok.\n"),
            (": six 6 . ;", "ok.\n"),
            (": nif if one if two two else six then one then ;", "ok.\n"),
            ("  0 nif", "ok.\n"),
            ("0 1 nif", "1 6 1 ok.\n"),
            ("1 1 nif", "1 2 2 1 ok.\n"),
            ("42 emit", "*ok.\n"),
            (": star 42 emit ;", "ok.\n"),
            ("star star star", "***ok.\n"),
        ];

        for (line, out) in lines {
            println!("{}", line);
            forth.input.fill(line).unwrap();
            forth.process_line().unwrap();
            print!(" => {}", forth.output.as_str());
            assert_eq!(forth.output.as_str(), *out);
            forth.output.clear();
        }

        forth.input.fill(": derp boop yay").unwrap();
        assert!(forth.process_line().is_err());
        // TODO: Should handle this automatically...
        forth.return_stack.clear();

        forth.input.fill(": doot yay yaay").unwrap();
        assert!(forth.process_line().is_err());
        // TODO: Should handle this automatically...
        forth.return_stack.clear();

        forth.output.clear();
        forth.input.fill("boop yay").unwrap();
        forth.process_line().unwrap();
        assert_eq!(forth.output.as_str(), "5 5 5 ok.\n");

        let mut any_stacks = false;

        while let Some(dsw) = forth.data_stack.pop() {
            println!("DSW: {:?}", dsw);
            any_stacks = true;
        }
        while let Some(rsw) = forth.return_stack.pop() {
            println!("RSW: {:?}", rsw);
            any_stacks = true;
        }
        assert!(!any_stacks);

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
