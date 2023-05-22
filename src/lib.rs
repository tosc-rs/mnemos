// For now...
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(any(test, feature = "use-std")), no_std)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod dictionary;
pub mod fastr;
pub mod input;
pub mod output;
pub mod stack;
pub(crate) mod vm;
pub mod word;

#[cfg(any(test, feature = "use-std"))]
pub mod leakbox;

use core::ptr::NonNull;

use dictionary::{BuiltinEntry, EntryHeader, EntryKind};

#[cfg(feature = "async")]
use dictionary::AsyncBuiltinEntry;

pub use crate::vm::Forth;
#[cfg(feature = "async")]
pub use crate::vm::AsyncForth;
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
    AddrOfMissingName,
    AddrOfNotAWord,

    // Not *really* an error - but signals that a function should be called
    // again. At the moment, only used for internal interpreter functions.
    PendingCallAgain,
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
    fn from(_oe: core::fmt::Error) -> Self {
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
        let req_start = self.idx;
        let req_end = req_start + n;
        if req_end > self.len {
            return Err(Error::BadCfaOffset);
        }
        let eh = unsafe { self.eh.as_ref() };
        match eh.kind {
            EntryKind::StaticBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::RuntimeBuiltin => Err(Error::BuiltinHasNoNextValue),
            #[cfg(feature = "async")]
            EntryKind::AsyncBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::Dictionary => unsafe {
                let de = self.eh.cast::<DictionaryEntry<T>>();
                let start = DictionaryEntry::pfa(de).as_ptr().add(req_start as usize);
                Ok(core::slice::from_raw_parts(start, n as usize))
            },
        }
    }

    fn get_current_val(&self) -> Result<i32, Error> {
        let w = self.get_current_word()?;
        Ok(unsafe { w.data })
    }

    fn get_current_word(&self) -> Result<Word, Error> {
        if self.idx >= self.len {
            return Err(Error::BadCfaOffset);
        }
        let eh = unsafe { self.eh.as_ref() };
        match eh.kind {
            EntryKind::StaticBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::RuntimeBuiltin => Err(Error::BuiltinHasNoNextValue),
            #[cfg(feature = "async")]
            EntryKind::AsyncBuiltin => Err(Error::BuiltinHasNoNextValue),
            EntryKind::Dictionary => unsafe {
                let de = self.eh.cast::<DictionaryEntry<T>>();
                let val_ptr = DictionaryEntry::pfa(de).as_ptr().add(self.idx as usize);
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

    fn get_word_at_cur_idx(&self) -> Option<&Word> {
        if self.idx >= self.len {
            return None;
        }
        let eh = unsafe { self.eh.as_ref() };
        match eh.kind {
            EntryKind::StaticBuiltin => None,
            EntryKind::RuntimeBuiltin => None,
            #[cfg(feature = "async")]
            EntryKind::AsyncBuiltin => None,
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
    Dict {
        de: NonNull<DictionaryEntry<T>>,
    },
    Literal {
        val: i32,
    },
    #[cfg(feature = "floats")]
    LiteralF {
        val: f32,
    },
    Builtin {
        bi: NonNull<BuiltinEntry<T>>,
    },
    #[cfg(feature = "async")]
    Async {
        bi: NonNull<AsyncBuiltinEntry<T>>,
    },
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
    use core::{future::Future, cmp::Ordering, task::Poll};

    use crate::{
        dictionary::DictionaryEntry,
        leakbox::{LBForth, LBForthParams},
        word::Word,
        Forth,
        Error,
    };

    #[derive(Default)]
    struct TestContext {
        contents: Vec<i32>,
    }

    #[test]
    fn sizes() {
        use core::mem::{align_of, size_of};
        assert_eq!(5 * size_of::<usize>(), size_of::<DictionaryEntry<()>>());
        assert_eq!(5 * size_of::<usize>(), size_of::<DictionaryEntry<()>>());
        assert_eq!(1 * size_of::<usize>(), align_of::<Word>());
    }

    #[test]
    fn forth() {
        let mut lbforth = LBForth::from_params(
            LBForthParams::default(),
            TestContext::default(),
            Forth::<TestContext>::FULL_BUILTINS,
        );

        test_forth(&mut lbforth.forth,|forth| forth.process_line(), |forth| forth);

        let context = lbforth.forth.release();
        assert_eq!(&context.contents, &[6, 5, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    fn test_lines(name: &str, forth: &mut Forth<TestContext>, lines: &[(&str, &str)]) {
        let pad = if name.is_empty() {
            ""
        } else {
            ": "
        };
        for (line, out) in lines {
            println!("{name}{pad}{line}");
            forth.input.fill(line).unwrap();
            forth.process_line().unwrap();
            print!("{name}{pad}=> {}", forth.output.as_str());
            assert_eq!(forth.output.as_str(), *out);
            forth.output.clear();
        }
    }

    // TODO: This test puns the heap-allocated cell into an array of bytes. This causes
    // miri to complain that the write is without provenance, which, fair. We might want
    // to look into some way of handling this particularly to be able to safely deal with
    // ALLOT and other alloca-style bump allocations of variable-sized data.
    #[cfg(not(miri))]
    #[test]
    fn ptr_math() {
        let mut lbforth = LBForth::from_params(
            LBForthParams::default(),
            TestContext::default(),
            Forth::<TestContext>::FULL_BUILTINS,
        );

        let forth = &mut lbforth.forth;

        test_lines("", forth, &[
            // declare a variable
            ("variable ptrword", "ok.\n"),
            // write an initial value `0x76543210` -> 1985229328
            ("1985229328 ptrword !", "ok.\n"),
            // Make sure it worked
            ("ptrword @ .", "1985229328 ok.\n"),
            // this assumes little endian lol sorry
            (": reader 4 0 do ptrword i + b@ . loop ;", "ok.\n"),
            // 0x10, 0x32, 0x54, 0x76
            ("reader", "16 50 84 118 ok.\n"),
            //                |------------| x = ptrword[i]
            //                               |-| x += i
            //                                   | -----------| ptrword[i] = x
            (": writer 4 0 do i ptrword + b@ i + ptrword i + b! loop ;", "ok.\n"),
            ("writer", "ok.\n"),
            // 0x10, 0x33, 0x56, 0x79
            ("reader", "16 51 86 121 ok.\n"),
        ]);
    }

    #[test]
    fn execute() {
        let mut lbforth = LBForth::from_params(
            LBForthParams::default(),
            TestContext::default(),
            Forth::<TestContext>::FULL_BUILTINS,
        );

        let forth = &mut lbforth.forth;

        test_lines("", forth, &[
            // define two words
            (": hello .\" hello, world!\" ;", "ok.\n"),
            (": goodbye .\" goodbye, world!\" ;", "ok.\n"),
            // take their addresses
            ("' goodbye", "ok.\n"),
            ("' hello", "ok.\n"),
            // and exec them!
            ("execute", "hello, world!ok.\n"),
            ("execute", "goodbye, world!ok.\n"),
        ]);
    }

    struct CountingFut<'forth> {
        target: usize,
        ctr: usize,
        forth: &'forth mut Forth<TestContext>,
    }

    impl<'forth> Future for CountingFut<'forth> {
        type Output = Result<(), Error>;

        fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
            match self.ctr.cmp(&self.target) {
                Ordering::Less => {
                    self.ctr += 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                },
                Ordering::Equal => {
                    self.ctr += 1;
                    let word = Word::data(self.ctr as i32);
                    self.forth.data_stack.push(word)?;
                    Poll::Ready(Ok(()))
                },
                Ordering::Greater => {
                    Poll::Ready(Err(Error::InternalError))
                },
            }
        }
    }

    #[cfg(feature = "async")]
    #[test]
    fn async_forth() {
        use crate::{dictionary::{AsyncBuiltins, AsyncBuiltinEntry}, fastr::FaStr, async_builtin, leakbox::AsyncLBForth};

        struct TestAsyncDispatcher;
        impl<'forth> AsyncBuiltins<'forth, TestContext> for TestAsyncDispatcher {
            type Future = CountingFut<'forth>;

            const BUILTINS: &'static [AsyncBuiltinEntry<TestContext>] = &[
                async_builtin!("counter"),
            ];

            fn dispatch_async(
                &self,
                id: &FaStr,
                forth: &'forth mut Forth<TestContext>,
            ) -> Self::Future {
                match id.as_str() {
                    "counter" => {
                        // Get value from top of stack
                        let val: usize = forth.data_stack.pop().unwrap().try_into().unwrap();
                        CountingFut { ctr: 0, target: val, forth }
                    }
                    id => panic!("Unknown async builtin {id}")
                }
            }
        }

        let mut lbforth = AsyncLBForth::from_params(
            LBForthParams::default(),
            TestContext::default(),
            Forth::<TestContext>::FULL_BUILTINS,
            TestAsyncDispatcher,
        );
        let forth = &mut lbforth.forth;

        let lines = &[
            ("5 counter", "ok.\n"),
        ];

        for (line, out) in lines {
            println!("{}", line);
            forth.input_mut().fill(line).unwrap();
            futures::executor::block_on(forth.process_line()).unwrap();
            print!(" => {}", forth.output().as_str());
            assert_eq!(forth.output().as_str(), *out);
            forth.output_mut().clear();
        }
    }

    #[cfg(feature = "async")]
    #[test]
    fn async_forth_not() {
        use crate::{dictionary::{AsyncBuiltins, AsyncBuiltinEntry}, fastr::FaStr, leakbox::AsyncLBForth, AsyncForth};

        struct TestAsyncDispatcher;
        impl<'forth> AsyncBuiltins<'forth, TestContext> for TestAsyncDispatcher {
            type Future = futures::future::Ready<Result<(), Error>>;
            const BUILTINS: &'static [AsyncBuiltinEntry<TestContext>] = &[];
            fn dispatch_async(
                &self,
                _id: &FaStr,
                _forth: &'forth mut Forth<TestContext>,
            ) -> Self::Future {
                 unreachable!("no async builtins should be called in this test")
            }
        }

        let mut lbforth = AsyncLBForth::from_params(
            LBForthParams::default(),
            TestContext::default(),
            Forth::<TestContext>::FULL_BUILTINS,
            TestAsyncDispatcher);
        test_forth(&mut lbforth.forth, |forth| futures::executor::block_on(forth.process_line()), AsyncForth::vm_mut)
    }

    fn test_forth<T>(forth: &mut T, process_line: impl Fn(&mut T) -> Result<(), Error>, get_forth: impl Fn(&mut T) -> &mut Forth<TestContext>) {
        assert_eq!(0, get_forth(forth).dict_alloc.used());
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
            ("forget z", "ok.\n"),
            ("variable a", "ok.\n"),
            ("100 a !", "ok.\n"),
            ("array z 4", "ok.\n"),
            ("z @ . z 1 w+ @ . z 2 w+ @ . z 3 w+ @ .", "0 0 0 0 ok.\n"),
        ];

        for (line, out) in lines {
            println!("{}", line);
            get_forth(forth).input.fill(line).unwrap();
            process_line(forth).unwrap();
            print!(" => {}", get_forth(forth).output.as_str());
            assert_eq!(get_forth(forth).output.as_str(), *out);
            get_forth(forth).output.clear();
        }

        get_forth(forth).input.fill(": derp boop yay").unwrap();
        assert!(process_line(forth).is_err());
        // TODO: Should handle this automatically...
        get_forth(forth).return_stack.clear();

        get_forth(forth).input.fill(": doot yay yaay").unwrap();
        assert!(process_line(forth).is_err());
        // TODO: Should handle this automatically...
        get_forth(forth).return_stack.clear();

        get_forth(forth).output.clear();
        get_forth(forth).input.fill("boop yay").unwrap();
        process_line(forth).unwrap();
        assert_eq!(get_forth(forth).output.as_str(), "5 5 5 ok.\n");

        let mut any_stacks = false;

        while let Some(dsw) = get_forth(forth).data_stack.pop() {
            println!("DSW: {:?}", dsw);
            any_stacks = true;
        }
        while let Some(rsw) = get_forth(forth).return_stack.pop() {
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
        get_forth(forth).add_builtin("squirrel", squirrel).unwrap();

        let lines = &[
            ("5 6 squirrel squirrel", "ok.\n"),
            (": sqloop 10 0 do i squirrel loop ;", "ok.\n"),
            ("sqloop", "ok.\n"),
        ];

        get_forth(forth).output.clear();
        for (line, out) in lines {
            println!("{}", line);
            get_forth(forth).input.fill(line).unwrap();
            process_line(forth).unwrap();
            print!(" => {}", get_forth(forth).output.as_str());
            assert_eq!(get_forth(forth).output.as_str(), *out);
            get_forth(forth).output.clear();
        }
    }
}
