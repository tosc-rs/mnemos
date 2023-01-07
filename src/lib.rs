// For now...
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(any(test, feature = "use-std")), no_std)]

pub mod dictionary;
pub mod fastr;
pub mod input;
pub mod output;
pub mod stack;
pub mod vm;
pub mod word;

#[cfg(any(test, feature = "use-std"))]
pub mod leakbox;

use core::ptr::NonNull;

use dictionary::{BuiltinEntry, EntryHeader, EntryKind};

pub use crate::vm::Forth;
use crate::{
    dictionary::{BumpError, DictionaryEntry},
    output::OutputError,
    stack::StackError,
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
    LoopCountIsNegative,
    LQuoteMissingRQuote,
    LiteralStringTooLong,
    NullPointerInCFA,
    BadStrLiteral,
    ForgetWithoutWordName,
    ForgetNotInDict,
    CantForgetBuiltins,
    InternalError,
    BadLiteral,
    BadWordOffset,
    BadArrayLength,
    DivideByZero,
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

impl From<core::fmt::Error> for Error {
    fn from(oe: core::fmt::Error) -> Self {
        Error::Output(OutputError::FormattingErr)
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
    pub(crate) fn get_next_n_words(&self, n: u16) -> Result<&[Word], Error> {
        let req_start = self.idx + 1;
        let req_end = req_start + n;
        if req_end > self.len {
            return Err(Error::BadCfaOffset);
        }
        let eh = unsafe { self.eh.as_ref() };
        match eh.kind {
            EntryKind::StaticBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::RuntimeBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::Dictionary => unsafe {
                let de = self.eh.cast::<DictionaryEntry<T>>();
                let start = DictionaryEntry::pfa(de).as_ptr().add(req_start as usize);
                Ok(core::slice::from_raw_parts(start, n as usize))
            },
        }
    }

    fn get_next_val(&self) -> Result<i32, Error> {
        let w = self.get_next_word()?;
        Ok(unsafe { w.data })
    }

    fn get_next_word(&self) -> Result<Word, Error> {
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
                let val_ptr = DictionaryEntry::pfa(de).as_ptr().add(req as usize);
                let val = val_ptr.read();
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

pub enum Lookup<T: 'static> {
    Dict { de: NonNull<DictionaryEntry<T>> },
    Literal { val: i32 },
    LiteralF { val: f32 },
    Builtin { bi: NonNull<BuiltinEntry<T>> },
    LQuote,
    LParen,
    Semicolon,
    If,
    Else,
    Then,
    Do,
    Loop,
    Constant,
    Variable,
    Array,
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
            (": beep .\" hello, world!\" ;", "ok.\n"),
            ("beep", "hello, world!ok.\n"),
            ("constant x 123", "ok.\n"),
            ("x .", "123 ok.\n"),
            ("4 x + .", "127 ok.\n"),
            ("variable y", "ok.\n"),
            ("y @ .", "0 ok.\n"),
            ("10 y !", "ok.\n"),
            ("y @ .", "10 ok.\n"),
            ("array z 4", "ok.\n"),
            ("z @ . z 1 w+ @ . z 2 w+ @ . z 3 w+ @ .", "0 0 0 0 ok.\n"),
            ("10 z ! 20 z 1 w+ ! 30 z 2 w+ ! 40 z 3 w+ !", "ok.\n"),
            (
                "z @ . z 1 w+ @ . z 2 w+ @ . z 3 w+ @ .",
                "10 20 30 40 ok.\n",
            ),
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

        let context = lbforth.forth.release();
        assert_eq!(&context.contents, &[6, 5, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }
}
