use core::{
    mem::size_of,
    num::NonZeroU16,
    ops::{Deref, Neg},
    ptr::NonNull,
    str::FromStr,
};

use crate::{
    dictionary::{
        DictLocation, BuiltinEntry, BumpError, DictionaryEntry, EntryHeader,
        EntryKind, OwnedDict,
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
pub struct Forth<T: 'static> {
    mode: Mode,
    pub data_stack: Stack<Word>,
    pub(crate) return_stack: Stack<Word>,
    pub(crate) call_stack: Stack<CallContext<T>>,
    pub(crate) dict: OwnedDict<T>,
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
        dict: OwnedDict<T>,
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
        builtins: &'static [BuiltinEntry<T>],
    ) -> Result<Self, Error> {
        let data_stack = Stack::new(dstack_buf.0, dstack_buf.1);
        let return_stack = Stack::new(rstack_buf.0, rstack_buf.1);
        let call_stack = Stack::new(cstack_buf.0, cstack_buf.1);

        Ok(Self {
            mode: Mode::Run,
            data_stack,
            return_stack,
            call_stack,
            dict,
            input,
            output,
            host_ctxt,
            builtins,

            #[cfg(feature = "async")]
            async_builtins: &[],
        })
    }

    /// Pushes a task to the back of the local queue, skipping the LIFO
    /// slot, and overflowing onto the injection queue if the local
    /// queue is full.
    #[cfg(feature = "async")]
     unsafe fn new_async(
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        cstack_buf: (*mut CallContext<T>, usize),
        dict: OwnedDict<T>,
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
        builtins: &'static [BuiltinEntry<T>],
        async_builtins: &'static [AsyncBuiltinEntry<T>],
    ) -> Result<Self, Error> {
        let data_stack = Stack::new(dstack_buf.0, dstack_buf.1);
        let return_stack = Stack::new(rstack_buf.0, rstack_buf.1);
        let call_stack = Stack::new(cstack_buf.0, cstack_buf.1);

        Ok(Self {
            mode: Mode::Run,
            data_stack,
            return_stack,
            call_stack,
            dict,
            input,
            output,
            host_ctxt,
            builtins,
            async_builtins,
        })
    }

    /// Constructs a new VM whose dictionary is a fork of this VM's dictionary.
    ///
    /// The current dictionary owned by this VM is frozen (made immutable), and
    /// a reference to it is shared with this VM and the new child VM. When both
    /// this VM and the child are dropped, the frozen dictionary is deallocated.
    ///
    /// This function takes two [`OwnedDict`]s as arguments: `new_dict` is the
    /// dictionary allocation for the forked child VM, while `my_dict` is a new
    /// allocation for this VM's mutable dictionary (which replaces the current
    /// dictionary, as it will become frozen).
    ///
    /// The child VM is created with empty stacks, and the provided input and
    /// output buffers.
    ///
    /// # Safety
    ///
    /// This method requires the same invariants be upheld as [`Forth::new`].
    pub unsafe fn fork(
        &mut self,
        mut new_dict: OwnedDict<T>,
        my_dict: OwnedDict<T>,
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        cstack_buf: (*mut CallContext<T>, usize),
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
    ) -> Result<Self, Error> {
        let shared_dict = self.dict.fork_onto(my_dict);
        new_dict.set_parent(shared_dict);
        Self::new(
            dstack_buf,
            rstack_buf,
            cstack_buf,
            new_dict,
            input,
            output,
            host_ctxt,
            self.builtins,
        )
    }

    pub fn add_builtin_static_name(
        &mut self,
        name: &'static str,
        bi: WordFunc<T>,
    ) -> Result<(), Error> {
        let name = unsafe { FaStr::new(name.as_ptr(), name.len()) };
        self.dict.add_bi_fastr(name, bi)?;
        Ok(())
    }

    pub fn add_builtin(&mut self, name: &str, bi: WordFunc<T>) -> Result<(), Error> {
        let name = self.dict.alloc.bump_str(name)?;
        self.dict.add_bi_fastr(name, bi)?;
        Ok(())
    }

    fn parse_num(word: &str) -> Option<i32> {
        i32::from_str(word).ok()
    }

    fn find_word(&self, word: &str) -> Option<NonNull<EntryHeader<T>>> {
        let fastr = TmpFaStr::new_from(word);
        self.find_in_dict(&fastr)
            .map(|entry| match entry {
                DictLocation::Current(entry) => entry.cast(),
                DictLocation::Parent(entry) => entry.cast(),
            })
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

    fn find_in_dict(&self, fastr: &TmpFaStr<'_>) -> Option<DictLocation<T>> {
        self.dict.entries()
            .find(|de| &unsafe { de.entry().as_ref() }.hdr.name == fastr.deref())
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
                    return Ok(Lookup::Dict(entry));
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
            // Found in the current dictionary, so call it.
            Lookup::Dict(DictLocation::Current(de)) => {
                let dref = unsafe { de.as_ref() };
                self.call_stack.push(CallContext {
                    eh: de.cast(),
                    idx: 0,
                    len: dref.hdr.len,
                })?;

                return Ok(ProcessAction::Execute);
            }
            // Found in a parent (frozen) dictionary. If this is a variable, we
            // may mutate it, so it must be copied into our dictionary.
            // TODO(eliza): we probably only need to do this when it's a
            // variable lookup?
            Lookup::Dict(DictLocation::Parent(de)) => {
                let dref = unsafe { de.as_ref() };
                let mut builder = self.dict.build_entry()?;
                unsafe {
                    let mut p = DictionaryEntry::pfa(de).as_ptr();
                    for _ in 0..dref.hdr.len {
                        builder = builder.write_word(p.read())?;
                        p = p.offset(1);
                    }
                }
                let name = unsafe {
                    // safety: a `FaStr` points to a string region stored in a
                    // dictionary. we can alias the name because our dictionary
                    // holds a reference to the parent dictionary, keeping it
                    // alive as long as our dictionary exists, and the new
                    // pointer will be in a value in our dictionary.
                    //
                    // IF IT WAS POSSIBLE FOR PARENTS TO BE DROPPED WHILE THEIR
                    // FORKS EXIST, THIS WOULD BE A DANGLING POINTER. IF YOU
                    // EVER CHANGE THE PARENT REFERENCE COUNTING RULES TO ALLOW
                    // PARENTS TO BE DEALLOCATED WHILE A CHILD EXISTS, YOU MUST
                    // CHANGE THIS TO DEEP COPY THE `FaStr` INTO THE CHILD
                    // DICT'S ARENA.
                    dref.hdr.name.copy_in_child()
                };
                let entry = builder.kind(dref.hdr.kind).finish(name, dref.func);
                self.call_stack.push(CallContext {
                    eh: entry.cast(),
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
        self.dict.alloc.bump_write(Word::ptr(literal_cj.as_ptr()))?;
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
        self.dict.alloc
            .bump_write(Word::ptr(literal_dojmp.as_ptr()))?;
        self.dict.alloc.bump_write(Word::data(offset))?;
        *len += 2;

        Ok(*len - start)
    }

    fn munch_if(&mut self, len: &mut u16) -> Result<u16, Error> {
        let start = *len;

        // Write a conditional jump, followed by space for a literal
        let literal_cj = self.find_word("(jump-zero)").ok_or(Error::WordNotInDict)?;
        self.dict.alloc.bump_write(Word::ptr(literal_cj.as_ptr()))?;
        let cj_offset: &mut i32 = {
            let cj_offset_word = self.dict.alloc.bump::<Word>()?;
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
        self.dict.alloc
            .bump_write(Word::ptr(literal_jmp.as_ptr()))?;
        let jmp_offset: &mut i32 = {
            let jmp_offset_word = self.dict.alloc.bump::<Word>()?;
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
            Lookup::Dict(DictLocation::Current(de)) | Lookup::Dict(DictLocation::Parent(de)) => {
                // Dictionary items are put into the CFA array directly as
                // a pointer to the dictionary entry
                self.dict.alloc.bump_write(Word::ptr(de.as_ptr()))?;
                *len += 1;
            }
            Lookup::Builtin { bi } => {
                self.dict.alloc.bump_write(Word::ptr(bi.as_ptr()))?;
                *len += 1;
            }
            #[cfg(feature = "async")]
            Lookup::Async { bi } => {
                self.dict.alloc.bump_write(Word::ptr(bi.as_ptr()))?;
                *len += 1;
            }
            #[cfg(feature = "floats")]
            Lookup::LiteralF { val } => {
                // Literals are added to the CFA as two items:
                //
                // 1. The address of the `literal()` dictionary item
                // 2. The value of the literal, as a data word
                let literal_dict = self.find_word("(literal)").ok_or(Error::WordNotInDict)?;
                self.dict.alloc
                    .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                self.dict.alloc.bump_write(Word::float(val))?;
                *len += 2;
            }
            Lookup::Literal { val } => {
                // Literals are added to the CFA as two items:
                //
                // 1. The address of the `literal()` dictionary item
                // 2. The value of the literal, as a data word
                let literal_dict = self.find_word("(literal)").ok_or(Error::WordNotInDict)?;
                self.dict.alloc
                    .bump_write(Word::ptr(literal_dict.as_ptr()))?;
                self.dict.alloc.bump_write(Word::data(val))?;
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
        self.dict.alloc
            .bump_write::<Word>(Word::ptr(literal_writestr.as_ptr()))?;
        self.dict.alloc
            .bump_write::<Word>(Word::data(str_len.into()))?;
        *len += 2;

        let start_ptr = self
            .dict.alloc
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

    /// Take the next token off of the input buffer as a name, and allocate the
    /// name in the dictionary.
    fn munch_name(&mut self) -> Result<FaStr, Error> {
        self.input.advance();
        let name = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        self.dict.alloc.bump_str(name).map_err(Into::into)
    }

    // constant NAME VALUE
    fn munch_constant(&mut self, _len: &mut u16) -> Result<u16, Error> {
        let name = self.munch_name()?;

        self.input.advance();
        let value = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let value_i32 = value.parse::<i32>().replace_err(Error::BadLiteral)?;

        self.dict.build_entry()?.write_word(Word::data(value_i32))?
            // TODO: Should we look up `(constant)` for consistency?
            // Use `find_word`?
            .finish(name, Self::constant);
        Ok(0)
    }

    // variable NAME
    fn munch_variable(&mut self, _len: &mut u16) -> Result<u16, Error> {
        let name = self.munch_name()?;
        self.dict.build_entry()?.write_word(Word::data(0))?
            // TODO: Should we look up `(variable)` for consistency?
            // Use `find_word`?
            .finish(name, Self::variable);
        Ok(0)
    }

    // array NAME COUNT
    fn munch_array(&mut self, _len: &mut u16) -> Result<u16, Error> {
        let name = self.munch_name()?;

        self.input.advance();
        let count = self
            .input
            .cur_word()
            .ok_or(Error::ColonCompileMissingName)?;
        let count_u16 = count
            .parse::<NonZeroU16>()
            .replace_err(Error::BadArrayLength)?;

        let mut entry = self.dict.build_entry()?;
        for _ in 0..u16::from(count_u16) {
            entry = entry.write_word(Word::data(0))?;
        }
        // TODO: Should arrays push length and ptr? Or just ptr?
        //
        // TODO: Should we look up `(variable)` for consistency?
        // Use `find_word`?
        entry.finish(name, Self::variable);
        Ok(0)
    }
}

/// # Safety
///
/// A `Forth` VM contains raw pointers. However, these raw pointers point into
/// regions which are exclusively owned by the `Forth` VM, and they are only
/// mutably dereferenced by methods which take ownership over the Forth VM. The
/// Constructing a new VM via `Forth::new` is unsafe, as the caller is
/// responsible for ensuring that the pointed memory regions are exclusively
/// owned by the `Forth` VM and that they live at least as long as the VM does,
/// but as long as those invariants are upheld, the VM may be shared across
/// thread boundaries.
// TODO(eliza): it would be nicer if there was a way to have a version of
// `LBForth` or something that bundles a `Forth` VM together with its owned
// buffers, but without requiring `liballoc`...idk what that would look like.
unsafe impl<T: Send> Send for Forth<T> {}
unsafe impl<T: Sync> Sync for Forth<T> {}