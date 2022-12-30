// For now...
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(any(test, feature = "use-std")), no_std)]

pub mod dictionary;
pub mod fastr;
pub mod input;
pub mod output;
pub mod stack;
pub mod word;

#[cfg(any(test, feature = "use-std"))]
pub mod leakbox;

use core::{
    fmt::Write,
    ops::{Deref, Neg},
    ptr::NonNull,
    str::FromStr,
};

use dictionary::{BuiltinEntry, EntryHeader, EntryKind};
use fastr::comptime_fastr;

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
    CFANotInDict(Word),
    WordNotInDict,
    ColonCompileMissingName,
    ColonCompileMissingSemicolon,
    LookupFailed,
    WordToUsizeInvalid(i32),
    UsizeToWordInvalid(usize),
    ElseBeforeIf,
    ThenBeforeIf,
    IfWithoutThen,
    DuplicateElse,
    IfElseWithoutThen,
    CallStackCorrupted,
    InterpretingCompileOnlyWord,
    BadCfaOffset,
    LoopBeforeDo,
    DoWithoutLoop,
    BadCfaLen,
    BuiltinHasNoNextValue,
    UntaggedCFAPtr,
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

pub struct CallContext<T: 'static> {
    eh: NonNull<EntryHeader<T>>,
    idx: u16,
    len: u16,
}

impl<T: 'static> Clone for CallContext<T> {
    fn clone(&self) -> Self {
        Self {
            eh: self.eh,
            idx: self.idx,
            len: self.len,
        }
    }
}

impl<T: 'static> Copy for CallContext<T> {}

impl<T: 'static> CallContext<T> {
    fn get_next_val(&self) -> Result<i32, Error> {
        let req = self.idx + 1;
        if req >= self.len {
            return Err(Error::BadCfaOffset);
        }
        let eh = unsafe { self.eh.as_ref() };
        match eh.kind {
            EntryKind::StaticBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::RuntimeBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::Dictionary => unsafe {
                let de = self.eh.cast::<DictionaryEntry<T>>();
                let val = (*DictionaryEntry::pfa(de).as_ptr().add(req as usize)).data;
                Ok(val)
            },
        }
    }

    fn offset(&mut self, offset: i32) -> Result<(), Error> {
        let new_idx = i32::from(self.idx).wrapping_add(offset);
        self.idx = match u16::try_from(new_idx) {
            Ok(new) => new,
            Err(_) => return Err(Error::BadCfaOffset),
        };
        Ok(())
    }

    // fn cfa_arr(&self) -> &[Word] {
    //     unsafe { cfa_to_slice(self.cfa) }
    // }

    fn get_word_at_cur_idx(&self) -> Option<&Word> {
        if self.idx >= self.len {
            return None;
        }
        let eh = unsafe { self.eh.as_ref() };
        match eh.kind {
            EntryKind::StaticBuiltin => None,
            EntryKind::RuntimeBuiltin => None,
            EntryKind::Dictionary => unsafe {
                let de = self.eh.cast::<DictionaryEntry<T>>();
                Some(&*DictionaryEntry::pfa(de).as_ptr().add(self.idx as usize))
            },
        }
    }
}

/// `WordFunc` represents a function that can be used as part of a dictionary word.
///
/// It takes the current "full context" (e.g. `Fif`), as well as the CFA pointer
/// to the dictionary entry.
type WordFunc<T> = fn(&mut Forth<T>) -> Result<(), Error>;

/// Forth is the "context" of the VM/interpreter.
///
/// It does NOT include the input/output buffers, or any components that
/// directly rely on those buffers. This Forth context is composed with
/// the I/O buffers to create the `Fif` type. This is done for lifetime
/// reasons.
pub struct Forth<T: 'static> {
    mode: Mode,
    data_stack: Stack<Word>,
    return_stack: Stack<Word>,
    call_stack: Stack<CallContext<T>>,
    dict_alloc: DictionaryBump,
    run_dict_tail: Option<NonNull<DictionaryEntry<T>>>,
    pub input: WordStrBuf,
    pub output: OutputBuf,
    pub host_ctxt: T,
    builtins: &'static [BuiltinEntry<T>],

    // TODO: This will be for words that have compile time actions, I guess?
    _comp_dict_tail: Option<NonNull<DictionaryEntry<T>>>,
}

impl<T> Forth<T> {
    pub unsafe fn new(
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        cstack_buf: (*mut CallContext<T>, usize),
        dict_buf: (*mut u8, usize),
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
        builtins: &'static [BuiltinEntry<T>],
    ) -> Result<Self, Error> {
        let data_stack = Stack::new(dstack_buf.0, dstack_buf.1);
        let return_stack = Stack::new(rstack_buf.0, rstack_buf.1);
        let call_stack = Stack::new(cstack_buf.0, cstack_buf.1);
        let dict_alloc = DictionaryBump::new(dict_buf.0, dict_buf.1);

        Ok(Self {
            mode: Mode::Run,
            data_stack,
            return_stack,
            call_stack,
            dict_alloc,
            run_dict_tail: None,
            _comp_dict_tail: None,
            input,
            output,
            host_ctxt,
            builtins,
        })
    }

    pub fn add_builtin_static_name(
        &mut self,
        name: &'static str,
        bi: WordFunc<T>,
    ) -> Result<(), Error> {
        let name = unsafe { FaStr::new(name.as_ptr(), name.len()) };
        self.add_bi_fastr(name, bi)
    }

    pub fn add_builtin(&mut self, name: &str, bi: WordFunc<T>) -> Result<(), Error> {
        let name = self.dict_alloc.bump_str(name)?;
        self.add_bi_fastr(name, bi)
    }

    fn add_bi_fastr(&mut self, name: FaStr, bi: WordFunc<T>) -> Result<(), Error> {
        // Allocate and initialize the dictionary entry
        let dict_base = self.dict_alloc.bump::<DictionaryEntry<T>>()?;
        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
                hdr: EntryHeader {
                    func: bi,
                    name,
                    kind: EntryKind::RuntimeBuiltin,
                    len: 0,
                },
                link: self.run_dict_tail.take(),
                parameter_field: [],
            });
        }
        self.run_dict_tail = Some(dict_base);
        Ok(())
    }

    fn parse_num(word: &str) -> Option<i32> {
        i32::from_str(word).ok()
    }

    fn find_word(&self, word: &str) -> Option<NonNull<EntryHeader<T>>> {
        let fastr = TmpFaStr::new_from(word);
        self.find_in_dict(&fastr)
            .map(NonNull::cast)
            .or_else(|| self.find_in_bis(&fastr).map(NonNull::cast))
    }

    fn find_in_bis(&self, fastr: &TmpFaStr<'_>) -> Option<NonNull<BuiltinEntry<T>>> {
        self.builtins
            .iter()
            .find(|bi| &bi.hdr.name == fastr.deref())
            .map(NonNull::from)
    }

    fn find_in_dict(&self, fastr: &TmpFaStr<'_>) -> Option<NonNull<DictionaryEntry<T>>> {
        let mut optr: Option<NonNull<DictionaryEntry<T>>> = self.run_dict_tail;
        while let Some(ptr) = optr.take() {
            let de = unsafe { ptr.as_ref() };
            if &de.hdr.name == fastr.deref() {
                return Some(ptr);
            }
            optr = de.link;
        }
        None
    }

    pub fn lookup(&self, word: &str) -> Result<Lookup<T>, Error> {
        match word {
            ";" => Ok(Lookup::Semicolon),
            "if" => Ok(Lookup::If),
            "else" => Ok(Lookup::Else),
            "then" => Ok(Lookup::Then),
            "do" => Ok(Lookup::Do),
            "loop" => Ok(Lookup::Loop),
            _ => {
                let fastr = TmpFaStr::new_from(word);
                if let Some(entry) = self.find_in_dict(&fastr) {
                    Ok(Lookup::Dict { de: entry })
                } else if let Some(bis) = self.find_in_bis(&fastr) {
                    Ok(Lookup::Builtin { bi: bis })
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
                    let dref = unsafe { de.as_ref() };
                    self.call_stack.push(CallContext {
                        eh: de.cast(),
                        idx: 0,
                        len: dref.hdr.len,
                    })?;
                    let res = (dref.hdr.func)(self);
                    self.call_stack.pop().ok_or(Error::CallStackCorrupted)?;
                    res?;
                }
                Lookup::Builtin { bi } => {
                    // TODO: Do I want to push builtins to the call stack?
                    self.call_stack.push(CallContext { eh: bi.cast(), idx: 0, len: 0 })?;
                    let res = unsafe { (bi.as_ref().hdr.func)(self) };
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
                Lookup::Do => return Err(Error::InterpretingCompileOnlyWord),
                Lookup::Loop => return Err(Error::InterpretingCompileOnlyWord),
            }
        }
        writeln!(&mut self.output, "ok.").map_err(|_| OutputError::FormattingErr)?;
        Ok(())
    }

    pub fn release(self) -> T {
        self.host_ctxt
    }
}

/// `Fif` is an ephemeral container that holds both the Forth interpreter/VM
/// as well as the I/O buffers.
///
/// This was originally done to keep the lifetimes separate, so we could
/// mutate the I/O buffer (mostly popping values) while operating on the
/// forth VM. It may be possible to move `Fif`'s functionality back into the
/// `Forth` struct at a later point.
impl<T> Forth<T> {
    pub const FULL_BUILTINS: &'static [BuiltinEntry<T>] = &[
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("+"),
                func: Self::add,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("/"),
                func: Self::div,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("="),
                func: Self::equal,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("not"),
                func: Self::invert,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("mod"),
                func: Self::modu,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("dup"),
                func: Self::dup,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("i"),
                func: Self::loop_i,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("."),
                func: Self::pop_print,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr(":"),
                func: Self::colon,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("(literal)"),
                func: Self::literal,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("d>r"),
                func: Self::data_to_return_stack,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("2d>2r"),
                func: Self::data2_to_return2_stack,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("r>d"),
                func: Self::return_to_data_stack,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("(jump-zero)"),
                func: Self::jump_if_zero,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("(jmp)"),
                func: Self::jump,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("(jmp-doloop)"),
                func: Self::jump_doloop,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr("emit"),
                func: Self::emit,
                kind: EntryKind::StaticBuiltin,
                len: 0,
            },
        },
    ];

    fn skip_literal(&mut self) -> Result<(), Error> {
        let parent = self.call_stack.try_peek_back_n_mut(1)?;
        parent.offset(1)?;
        Ok(())
    }

    pub fn invert(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let val = if a == Word::data(0) {
            Word::data(-1)
        } else {
            Word::data(0)
        };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn equal(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = if a == b {
            Word::data(-1)
        } else {
            Word::data(0)
        };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn div(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = unsafe { Word::data(b.data / a.data) };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn modu(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = unsafe { Word::data(b.data % a.data) };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn loop_i(&mut self) -> Result<(), Error> {
        let a = self.return_stack.try_peek()?;
        self.data_stack.push(a)?;
        Ok(())
    }

    pub fn jump_doloop(&mut self) -> Result<(), Error> {
        let a = self.return_stack.try_pop()?;
        let b = self.return_stack.try_peek()?;
        let ctr = unsafe { Word::data(a.data + 1) };
        let do_jmp = ctr != b;
        if do_jmp {
            self.return_stack.push(ctr)?;
            self.jump()
        } else {
            self.return_stack.try_pop()?;
            self.skip_literal()
        }
    }

    pub fn emit(&mut self) -> Result<(), Error> {
        let val = self.data_stack.try_pop()?;
        let val = unsafe { val.data };
        self.output.push_bstr(&[val as u8])?;
        Ok(())
    }

    pub fn jump_if_zero(&mut self) -> Result<(), Error> {
        let do_jmp = unsafe {
            let val = self.data_stack.try_pop()?;
            val.data == 0
        };
        if do_jmp {
            self.jump()
        } else {
            self.skip_literal()
        }
    }

    pub fn jump(&mut self) -> Result<(), Error> {
        let parent = self.call_stack.try_peek_back_n_mut(1)?;
        let offset = parent.get_next_val()?;
        parent.offset(offset)?;
        Ok(())
    }

    pub fn dup(&mut self) -> Result<(), Error> {
        let val = self.data_stack.try_peek()?;
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn return_to_data_stack(&mut self) -> Result<(), Error> {
        let val = self.return_stack.try_pop()?;
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn data_to_return_stack(&mut self) -> Result<(), Error> {
        let val = self.data_stack.try_pop()?;
        self.return_stack.push(val)?;
        Ok(())
    }

    pub fn data2_to_return2_stack(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.return_stack.push(b)?;
        self.return_stack.push(a)?;
        Ok(())
    }

    pub fn pop_print(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        write!(&mut self.output, "{} ", unsafe { a.data })
            .map_err(|_| OutputError::FormattingErr)?;
        Ok(())
    }

    pub fn add(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
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
        let dict_base = self.dict_alloc.bump::<DictionaryEntry<T>>()?;

        let mut len = 0u16;

        // Begin compiling until we hit the end of the line or a semicolon.
        while self.munch_one(&mut len)? != 0 {}

        // Did we successfully get to the end, marked by a semicolon?
        if self.input.cur_word() == Some(";") {
            unsafe {
                dict_base.as_ptr().write(DictionaryEntry {
                    hdr: EntryHeader {
                        func: Self::interpret,
                        name,
                        kind: EntryKind::Dictionary,
                        len,
                    },
                    // Don't link until we know we have a "good" entry!
                    link: self.run_dict_tail.take(),
                    parameter_field: [],
                });
            }
            self.run_dict_tail = Some(dict_base);
            self.mode = old_mode;
            Ok(())
        } else {
            Err(Error::ColonCompileMissingSemicolon)
        }
    }

    fn munch_do(&mut self, len: &mut u16) -> Result<u16, Error> {
        let start = *len;

        // Write a conditional jump, followed by space for a literal
        let literal_cj = self.find_word("2d>2r").ok_or(Error::WordNotInDict)?;
        self.dict_alloc.bump_write(Word::ptr(literal_cj.as_ptr()))?;
        *len += 1;

        let do_start = *len;
        // Now work until we hit an else or then statement.
        loop {
            match self.munch_one(len) {
                // We hit the end of stream before an else/then.
                Ok(0) => return Err(Error::DoWithoutLoop),
                // We compiled some stuff, keep going...
                Ok(_) => {}
                Err(Error::LoopBeforeDo) => {
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        let delta = *len - do_start;
        let offset = i32::from(delta + 1).neg();
        let literal_dojmp = self.find_word("(jmp-doloop)").ok_or(Error::WordNotInDict)?;
        self.dict_alloc
            .bump_write(Word::ptr(literal_dojmp.as_ptr()))?;
        self.dict_alloc.bump_write(Word::data(offset))?;
        *len += 2;

        Ok(*len - start)
    }

    fn munch_if(&mut self, len: &mut u16) -> Result<u16, Error> {
        let start = *len;

        // Write a conditional jump, followed by space for a literal
        let literal_cj = self.find_word("(jump-zero)").ok_or(Error::WordNotInDict)?;
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
            *cj_offset = i32::from(delta) + 1;
            return Ok(*len - start);
        }
        // We got an "else", keep going for "then"
        //
        // Jump offset is words placed + 1 (cj lit) + 2 (else cj + lit)
        *cj_offset = i32::from(delta) + 3;

        // Write a conditional jump, followed by space for a literal
        let literal_jmp = self.find_word("(jmp)").ok_or(Error::WordNotInDict)?;
        self.dict_alloc.bump_write(Word::ptr(literal_jmp.as_ptr()))?;
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
        *jmp_offset = i32::from(delta) + 1;

        Ok(*len - start)
    }

    fn munch_one(&mut self, len: &mut u16) -> Result<u16, Error> {
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
                self.dict_alloc
                    .bump_write(Word::ptr(de.as_ptr()))?;
                *len += 1;
            }
            Lookup::Builtin { bi } => {
                self.dict_alloc
                    .bump_write(Word::ptr(bi.as_ptr()))?;
                *len += 1;
            }
            Lookup::Literal { val } => {
                // Literals are added to the CFA as two items:
                //
                // 1. The address of the `literal()` dictionary item
                // 2. The value of the literal, as a data word
                let literal_dict = self.find_word("(literal)").ok_or(Error::WordNotInDict)?;
                self.dict_alloc.bump_write(Word::ptr(literal_dict.as_ptr()))?;
                self.dict_alloc.bump_write(Word::data(val))?;
                *len += 2;
            }
            Lookup::Do => return self.munch_do(len),
            Lookup::Loop => return Err(Error::LoopBeforeDo),
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
        let parent = self.call_stack.try_peek_back_n_mut(1)?;
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
        // NOTE: we DON'T use `Stack::try_peek_back_n_mut` because the callee
        // could pop off our item, which would lead to UB.
        let mut me = self.call_stack.peek().unwrap();

        // For the remaining words, we do a while-let loop instead of
        // a for-loop, as some words (e.g. literals) require advancing
        // to the next word.
        while let Some(word) = me.get_word_at_cur_idx() {
            // We can safely assume that all items in the list are pointers,
            // EXCEPT for literals, but those are handled manually below.
            let ptr = unsafe { word.ptr.cast::<EntryHeader<T>>() };
            let nn = NonNull::new(ptr).unwrap();
            let ehref = unsafe { nn.as_ref() };

            self.call_stack.overwrite_back_n(0, me)?;
            self.call_stack.push(CallContext {
                eh: nn,
                idx: 0,
                len: ehref.len,
            })?;
            let result = (ehref.func)(self);
            self.call_stack.pop().unwrap();
            result?;
            me = self.call_stack.peek().unwrap();

            me.offset(1)?;
            // TODO: If I want A4-style pausing here, I'd probably want to also
            // push dictionary locations to the stack (under the CFA), which
            // would allow for halting and resuming. Yield after loading "next",
            // right before executing the function itself. This would also allow
            // for cursed control flow
        }
        Ok(())
    }

    #[cfg(any(test, feature = "use-std"))]
    pub fn print_dump(&self) {
        // for BuiltinEntry { name, .. } in self.builtins {
        //     println!("( static builtin - '{}' )", name.as_str());
        // }
        // if let Some(link) = self.run_dict_tail {
        //     unsafe {
        //         DictionaryEntry::<T>::dump_recursive(link);
        //     }
        // }
    }
}

pub enum Lookup<T: 'static> {
    Dict { de: NonNull<DictionaryEntry<T>> },
    Literal { val: i32 },
    Builtin { bi: NonNull<BuiltinEntry<T>> },
    Semicolon,
    If,
    Else,
    Then,
    Do,
    Loop,
}

trait ReplaceErr {
    type OK;
    fn replace_err<NE>(self, t: NE) -> Result<Self::OK, NE>;
}

impl<T, OE> ReplaceErr for Result<T, OE> {
    type OK = T;
    #[inline]
    fn replace_err<NE>(self, e: NE) -> Result<Self::OK, NE> {
        match self {
            Ok(t) => Ok(t),
            Err(_e) => Err(e),
        }
    }
}

#[cfg(test)]
pub mod test {
    use crate::{
        dictionary::DictionaryEntry,
        leakbox::{LBForth, LBForthParams},
        word::Word,
        Forth,
    };

    #[derive(Default)]
    struct TestContext {
        contents: Vec<i32>,
    }

    #[test]
    fn forth() {
        use core::mem::{align_of, size_of};
        assert_eq!(5 * size_of::<usize>(), size_of::<DictionaryEntry<()>>());
        assert_eq!(5 * size_of::<usize>(), size_of::<DictionaryEntry<()>>());
        assert_eq!(1 * size_of::<usize>(), align_of::<Word>());

        let mut lbforth = LBForth::from_params(
            LBForthParams::default(),
            TestContext::default(),
            Forth::<TestContext>::FULL_BUILTINS,
        );
        let forth = &mut lbforth.forth;
        assert_eq!(0, forth.dict_alloc.used());
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
            (": sloop one 5 0 do star star loop six ;", "ok.\n"),
            ("sloop", "1 **********6 ok.\n"),
            (": count 10 0 do i . loop ;", "ok.\n"),
            ("count", "0 1 2 3 4 5 6 7 8 9 ok.\n"),
            (": smod 10 0 do i 3 mod not if star then loop ;", "ok.\n"),
            ("smod", "****ok.\n"),
        ];

        for (line, out) in lines {
            println!("{}", line);
            forth.input.fill(line).unwrap();
            forth.process_line().unwrap();
            print!(" => {}", forth.output.as_str());
            assert_eq!(forth.output.as_str(), *out);
            forth.output.clear();
        }

        // forth.input.fill(": derp boop yay").unwrap();
        // assert!(forth.process_line().is_err());
        // // TODO: Should handle this automatically...
        // forth.return_stack.clear();

        // forth.input.fill(": doot yay yaay").unwrap();
        // assert!(forth.process_line().is_err());
        // // TODO: Should handle this automatically...
        // forth.return_stack.clear();

        // forth.output.clear();
        // forth.input.fill("boop yay").unwrap();
        // forth.process_line().unwrap();
        // assert_eq!(forth.output.as_str(), "5 5 5 ok.\n");

        // let mut any_stacks = false;

        // while let Some(dsw) = forth.data_stack.pop() {
        //     println!("DSW: {:?}", dsw);
        //     any_stacks = true;
        // }
        // while let Some(rsw) = forth.return_stack.pop() {
        //     println!("RSW: {:?}", rsw);
        //     any_stacks = true;
        // }
        // assert!(!any_stacks);

        // Uncomment if you want to check how much of the dictionary
        // was used during a test run.
        //
        // assert_eq!(176, forth.dict_alloc.used());

        // Uncomment this if you want to see the output of the
        // forth run. TODO: Remove this once we implement the
        // output buffer.
        //
        // panic!("Test Passed! Manual inspection...");

        // Takes one value off the stack, and stores it in the vec
        fn squirrel(forth: &mut Forth<TestContext>) -> Result<(), crate::Error> {
            let val = forth.data_stack.try_pop()?;
            forth.host_ctxt.contents.push(unsafe { val.data });
            Ok(())
        }
        forth.add_builtin("squirrel", squirrel).unwrap();

        let lines = &[
            ("5 6 squirrel squirrel", "ok.\n"),
            (": sqloop 10 0 do i squirrel loop ;", "ok.\n"),
            ("sqloop", "ok.\n"),
        ];

        forth.output.clear();
        for (line, out) in lines {
            println!("{}", line);
            forth.input.fill(line).unwrap();
            forth.process_line().unwrap();
            print!(" => {}", forth.output.as_str());
            assert_eq!(forth.output.as_str(), *out);
            forth.output.clear();
        }

        forth.print_dump();
        // panic!();

        let context = lbforth.forth.release();
        assert_eq!(&context.contents, &[6, 5, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }
}
