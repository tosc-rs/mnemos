use core::{
    mem::size_of,
    num::NonZeroU16,
    ops::{Deref, Neg},
    ptr::NonNull,
    str::FromStr, marker::PhantomData,
};

use crate::{
    dictionary::{
        BuiltinEntry, BumpError, DictionaryBump, DictionaryEntry, EntryHeader, EntryKind,
    },
    fastr::{FaStr, TmpFaStr},
    input::WordStrBuf,
    output::OutputBuf,
    stack::{Stack, StackError},
    word::Word,
    CallContext, Error, Lookup, Mode, ReplaceErr, WordFunc,
};

#[cfg(feature = "async")]
use crate::dictionary::{AsyncBuiltinEntry, AsyncBuiltins};

pub mod builtins;

#[cfg(feature = "async")]
mod async_vm;

#[cfg(feature = "async")]
pub use self::async_vm::AsyncForth;

/// Forth is the "context" of the VM/interpreter.
///
/// It does NOT include the input/output buffers, or any components that
/// directly rely on those buffers. This Forth context is composed with
/// the I/O buffers to create the `Fif` type. This is done for lifetime
/// reasons.
pub struct Forth<T: 'static> {
    mode: Mode,
    pub data_stack: Stack<Word>,
    pub(crate) return_stack: Stack<Word>,
    pub(crate) call_stack: Stack<CallContext<T>>,
    pub(crate) dict_alloc: DictionaryBump,
    run_dict_tail: Option<NonNull<DictionaryEntry<T>>>,
    pub input: WordStrBuf,
    pub output: OutputBuf,
    pub host_ctxt: T,
    builtins: &'static [BuiltinEntry<T>],
    #[cfg(feature = "async")]
    async_builtins: &'static [AsyncBuiltinEntry<T>],
}

enum ProcessAction {
    Continue,
    Execute,
    Done,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Step {
    Done,
    NotDone,
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
            input,
            output,
            host_ctxt,
            builtins,

            #[cfg(feature = "async")]
            async_builtins: &[],
        })
    }

    #[cfg(feature = "async")]
     unsafe fn new_async(
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        cstack_buf: (*mut CallContext<T>, usize),
        dict_buf: (*mut u8, usize),
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
        builtins: &'static [BuiltinEntry<T>],
        async_builtins: &'static [AsyncBuiltinEntry<T>],
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
            input,
            output,
            host_ctxt,
            builtins,
            async_builtins,
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
                    name,
                    kind: EntryKind::RuntimeBuiltin,
                    len: 0,
                    _pd: PhantomData,
                },
                func: bi,
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

    #[cfg(feature = "async")]
    fn find_in_async_bis(&self, fastr: &TmpFaStr<'_>) -> Option<NonNull<AsyncBuiltinEntry<T>>> {
        self.async_builtins
            .iter()
            .find(|bi| &bi.hdr.name == fastr.deref())
            .map(NonNull::from)
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
            "(" => Ok(Lookup::LParen),
            "constant" => Ok(Lookup::Constant),
            "variable" => Ok(Lookup::Variable),
            "array" => Ok(Lookup::Array),
            r#".""# => Ok(Lookup::LQuote),
            _ => {
                let fastr = TmpFaStr::new_from(word);
                if let Some(entry) = self.find_in_dict(&fastr) {
                    return Ok(Lookup::Dict { de: entry });
                }
                if let Some(bis) = self.find_in_bis(&fastr) {
                    return Ok(Lookup::Builtin { bi: bis });
                }

                #[cfg(feature = "async")]
                if let Some(bi) = self.find_in_async_bis(&fastr) {
                    return Ok(Lookup::Async { bi });
                }

                if let Some(val) = Self::parse_num(word) {
                    return Ok(Lookup::Literal { val });
                }

                #[cfg(feature = "floats")]
                if let Ok(fv) = word.parse::<f32>() {
                    return Ok(Lookup::LiteralF { val: fv });
                }

                Err(Error::LookupFailed)
            }
        }
    }

    pub fn process_line(&mut self) -> Result<(), Error> {
        let res = (|| {
            loop {
                match self.start_processing_line()? {
                    ProcessAction::Done => {
                        self.output.push_str("ok.\n")?;
                        break Ok(());
                    },
                    ProcessAction::Continue => {},
                    ProcessAction::Execute =>
                        // Loop until execution completes.
                        while self.steppa_pig()? != Step::Done {},
                }
            }
        })();
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                self.data_stack.clear();
                self.return_stack.clear();
                self.call_stack.clear();
                Err(e)
            }
        }
    }

    /// Returns `true` if we must call `steppa_pig` until it returns `Ready`,
    /// false if not.
    fn start_processing_line(&mut self) -> Result<ProcessAction, Error> {
        self.input.advance();
        let word = match self.input.cur_word() {
            Some(w) => w,
            None => return Ok(ProcessAction::Done),
        };

        match self.lookup(word)? {
            Lookup::Dict { de } => {
                let dref = unsafe { de.as_ref() };
                self.call_stack.push(CallContext {
                    eh: de.cast(),
                    idx: 0,
                    len: dref.hdr.len,
                })?;

                return Ok(ProcessAction::Execute);
            }
            Lookup::Builtin { bi } => {
                self.call_stack.push(CallContext {
                    eh: bi.cast(),
                    idx: 0,
                    len: 0,
                })?;

                return Ok(ProcessAction::Execute);
            }
            #[cfg(feature = "async")]
            Lookup::Async { bi } => {
                self.call_stack.push(CallContext {
                    eh: bi.cast(),
                    idx: 0,
                    len: 0,
                })?;

                return Ok(ProcessAction::Execute);
            },
            Lookup::Literal { val } => {
                self.data_stack.push(Word::data(val))?;
            }
            #[cfg(feature = "floats")]
            Lookup::LiteralF { val } => {
                self.data_stack.push(Word::float(val))?;
            }
            Lookup::LParen => {
                self.munch_comment(&mut 0)?;
            }
            Lookup::Semicolon => return Err(Error::InterpretingCompileOnlyWord),
            Lookup::If => return Err(Error::InterpretingCompileOnlyWord),
            Lookup::Else => return Err(Error::InterpretingCompileOnlyWord),
            Lookup::Then => return Err(Error::InterpretingCompileOnlyWord),
            Lookup::Do => return Err(Error::InterpretingCompileOnlyWord),
            Lookup::Loop => return Err(Error::InterpretingCompileOnlyWord),
            Lookup::LQuote => {
                self.input.advance_str().replace_err(Error::BadStrLiteral)?;
                let lit = self.input.cur_str_literal().unwrap();
                self.output.push_str(lit)?;
            }
            Lookup::Constant => {
                self.munch_constant(&mut 0)?;
            }
            Lookup::Variable => {
                self.munch_variable(&mut 0)?;
            }
            Lookup::Array => {
                self.munch_array(&mut 0)?;
            }
        }

        Ok(ProcessAction::Continue)
    }

    // Single step execution
    fn steppa_pig(&mut self,) -> Result<Step, Error> {
        let top = match self.call_stack.try_peek() {
            Ok(t) => t,
            Err(StackError::StackEmpty) => return Ok(Step::Done),
            Err(e) => return Err(Error::Stack(e)),
        };

        let kind = unsafe { top.eh.as_ref().kind };
        let res = unsafe { match kind {
            EntryKind::StaticBuiltin => (top.eh.cast::<BuiltinEntry<T>>().as_ref().func)(self),
            EntryKind::RuntimeBuiltin => (top.eh.cast::<BuiltinEntry<T>>().as_ref().func)(self),
            EntryKind::Dictionary => (top.eh.cast::<DictionaryEntry<T>>().as_ref().func)(self),

            #[cfg(feature = "async")]
            EntryKind::AsyncBuiltin => {
                unreachable!(
                    "only an AsyncForth VM should have async builtins, and an \
                    AsyncForth VM should never perform a non-async execution \
                    step! this is a bug."
                )
            },
        }};

        match res {
            Ok(_) => {
                let _ = self.call_stack.pop();
            }
            Err(Error::PendingCallAgain) => {
                // ok, just don't pop
            }
            Err(e) => return Err(e),
        }

        Ok(Step::NotDone)
    }

    /// Interpret is the run-time target of the `:` (colon) word.
    pub fn interpret(&mut self) -> Result<(), Error> {
        let mut top = self.call_stack.try_peek()?;

        if let Some(word) = top.get_word_at_cur_idx() {
            // Push the item in the list to the top of stack, will be executed on next step
            let ptr = unsafe { word.ptr.cast::<EntryHeader<T>>() };
            let nn = NonNull::new(ptr).ok_or(Error::NullPointerInCFA)?;
            let ehref = unsafe { nn.as_ref() };
            let callee = CallContext {
                eh: nn,
                idx: 0,
                len: ehref.len,
            };

            // Increment to the next item
            top.offset(1)?;
            self.call_stack.overwrite_back_n(0, top)?;

            // Then add the callee on top of the currently interpreted word
            self.call_stack.push(callee)?;

            Err(Error::PendingCallAgain)
        } else {
            Ok(())
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
                self.dict_alloc.bump_write(Word::ptr(de.as_ptr()))?;
                *len += 1;
            }
            Lookup::Builtin { bi } => {
                self.dict_alloc.bump_write(Word::ptr(bi.as_ptr()))?;
                *len += 1;
            }
            #[cfg(feature = "async")]
            Lookup::Async { bi } => {
                self.dict_alloc.bump_write(Word::ptr(bi.as_ptr()))?;
                *len += 1;
            }
            #[cfg(feature = "floats")]
            Lookup::LiteralF { val } => {
                // Literals are added to the CFA as two items:
                //
                // 1. The address of the `literal()` dictionary item
                // 2. The value of the literal, as a data word
                let literal_dict = self.find_word("(literal)").ok_or(Error::WordNotInDict)?;
                self.dict_alloc
                    .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                self.dict_alloc.bump_write(Word::float(val))?;
                *len += 2;
            }
            Lookup::Literal { val } => {
                // Literals are added to the CFA as two items:
                //
                // 1. The address of the `literal()` dictionary item
                // 2. The value of the literal, as a data word
                let literal_dict = self.find_word("(literal)").ok_or(Error::WordNotInDict)?;
                self.dict_alloc
                    .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                self.dict_alloc.bump_write(Word::data(val))?;
                *len += 2;
            }
            Lookup::Do => return self.munch_do(len),
            Lookup::Loop => return Err(Error::LoopBeforeDo),
            Lookup::LParen => return self.munch_comment(len),
            Lookup::LQuote => return self.munch_str(len),
            Lookup::Constant => return self.munch_constant(len),
            Lookup::Variable => return self.munch_variable(len),
            Lookup::Array => return self.munch_array(len),
        }
        Ok(*len - start)
    }

    pub fn release(self) -> T {
        self.host_ctxt
    }

    fn munch_comment(&mut self, _len: &mut u16) -> Result<u16, Error> {
        loop {
            self.input.advance();
            match self.input.cur_word() {
                Some(s) => {
                    if s.ends_with(')') {
                        return Ok(0);
                    }
                }
                None => return Ok(0),
            }
        }
    }

    fn munch_str(&mut self, len: &mut u16) -> Result<u16, Error> {
        let start = *len;
        self.input
            .advance_str()
            .replace_err(Error::LQuoteMissingRQuote)?;
        let lit_str = self
            .input
            .cur_str_literal()
            .ok_or(Error::LQuoteMissingRQuote)?;
        let str_len =
            u16::try_from(lit_str.as_bytes().len()).replace_err(Error::LiteralStringTooLong)?;

        let literal_writestr = self.find_word("(write-str)").ok_or(Error::WordNotInDict)?;
        self.dict_alloc
            .bump_write::<Word>(Word::ptr(literal_writestr.as_ptr()))?;
        self.dict_alloc
            .bump_write::<Word>(Word::data(str_len.into()))?;
        *len += 2;

        let start_ptr = self
            .dict_alloc
            .bump_u8s(lit_str.as_bytes().len())
            .ok_or(Error::Bump(BumpError::OutOfMemory))?;

        unsafe {
            start_ptr
                .as_ptr()
                .copy_from_nonoverlapping(lit_str.as_bytes().as_ptr(), lit_str.as_bytes().len());
        }
        let word_size = size_of::<Word>();
        let words_written = (str_len as usize + (word_size - 1)) / word_size;
        *len += words_written as u16;

        Ok(*len - start)
    }

    // constant NAME VALUE
    fn munch_constant(&mut self, _len: &mut u16) -> Result<u16, Error> {
        self.input.advance();
        let name = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let name = self.dict_alloc.bump_str(name)?;

        self.input.advance();
        let value = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let value_i32 = value.parse::<i32>().replace_err(Error::BadLiteral)?;

        let dict_base = self.dict_alloc.bump::<DictionaryEntry<T>>()?;
        self.dict_alloc.bump_write(Word::data(value_i32))?;
        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
                hdr: EntryHeader {
                    name,
                    kind: EntryKind::Dictionary,
                    len: 1,
                    _pd: PhantomData,
                },
                // TODO: Should we look up `(constant)` for consistency?
                // Use `find_word`?
                func: Self::constant,
                // Don't link until we know we have a "good" entry!
                link: self.run_dict_tail.take(),
                parameter_field: [],
            });
        }
        self.run_dict_tail = Some(dict_base);
        Ok(0)
    }

    // variable NAME
    fn munch_variable(&mut self, _len: &mut u16) -> Result<u16, Error> {
        self.input.advance();
        let name = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let name = self.dict_alloc.bump_str(name)?;

        let dict_base = self.dict_alloc.bump::<DictionaryEntry<T>>()?;
        self.dict_alloc.bump_write(Word::data(0))?;
        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
                hdr: EntryHeader {
                    name,
                    kind: EntryKind::Dictionary,
                    len: 1,
                    _pd: PhantomData,
                },
                // TODO: Should we look up `(variable)` for consistency?
                // Use `find_word`?
                func: Self::variable,
                // Don't link until we know we have a "good" entry!
                link: self.run_dict_tail.take(),
                parameter_field: [],
            });
        }
        self.run_dict_tail = Some(dict_base);
        Ok(0)
    }

    // array NAME COUNT
    fn munch_array(&mut self, _len: &mut u16) -> Result<u16, Error> {
        self.input.advance();
        let name = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let name = self.dict_alloc.bump_str(name)?;

        self.input.advance();
        let count = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let count_u16 = count
            .parse::<NonZeroU16>()
            .replace_err(Error::BadArrayLength)?;

        let dict_base = self.dict_alloc.bump::<DictionaryEntry<T>>()?;

        for _ in 0..u16::from(count_u16) {
            self.dict_alloc.bump_write(Word::data(0))?;
        }

        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
                hdr: EntryHeader {
                    name,
                    kind: EntryKind::Dictionary,
                    len: count_u16.into(),
                    _pd: PhantomData
                },
                // TODO: Should arrays push length and ptr? Or just ptr?
                //
                // TODO: Should we look up `(variable)` for consistency?
                // Use `find_word`?
                func: Self::variable,

                // Don't link until we know we have a "good" entry!
                link: self.run_dict_tail.take(),
                parameter_field: [],
            });
        }
        self.run_dict_tail = Some(dict_base);
        Ok(0)
    }
}
