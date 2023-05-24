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

#[cfg(any(test, doctest, feature = "_force_test_utils"))]
pub mod testutil;

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
        Error, testutil::{all_runtest, blocking_runtest_with},
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

        let forth = &mut lbforth.forth;

        assert_eq!(0, forth.dict_alloc.used());

        blocking_runtest_with(forth, r#"
            > : yay 2 3 + . ;
            > : boop yay yay ;
        "#);

        blocking_runtest_with(forth, r#"
            x : derp boop yay
        "#);
        assert!(forth.return_stack.is_empty());

        blocking_runtest_with(forth, r#"
            x : doot yay yaay
        "#);
        assert!(forth.return_stack.is_empty());

        blocking_runtest_with(forth, r#"
            > boop yay
            < 5 5 5 ok.
        "#);
        assert!(forth.data_stack.is_empty());
        assert!(forth.call_stack.is_empty());

        // Uncomment if you want to check how much of the dictionary
        // was used during a test run.
        //
        // assert_eq!(176, forth.dict_alloc.used());

        // Takes one value off the stack, and stores it in the vec
        fn squirrel(forth: &mut Forth<TestContext>) -> Result<(), crate::Error> {
            let val = forth.data_stack.try_pop()?;
            forth.host_ctxt.contents.push(unsafe { val.data });
            Ok(())
        }
        forth.add_builtin("squirrel", squirrel).unwrap();

        blocking_runtest_with(forth, r#"
            > 5 6 squirrel squirrel
            < ok.
            > : sqloop 10 0 do i squirrel loop ;
            < ok.
            > sqloop
            < ok.
        "#);

        let expected = [6, 5, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_eq!(&expected, forth.host_ctxt.contents.as_slice());
    }

    // TODO: This test puns the heap-allocated cell into an array of bytes. This causes
    // miri to complain that the write is without provenance, which, fair. We might want
    // to look into some way of handling this particularly to be able to safely deal with
    // ALLOT and other alloca-style bump allocations of variable-sized data.
    #[cfg(not(miri))]
    #[test]
    fn ptr_math() {
        all_runtest(r#"
            ( declare a variable )
            > variable ptrword
            < ok.

            ( write an initial value `0x76543210` -> 1985229328 )
            > 1985229328 ptrword !
            < ok.

            ( Make sure it worked )
            > ptrword @ .
            < 1985229328 ok.

            ( this assumes little endian lol sorry )
            > : reader 4 0 do ptrword i + b@ . loop ;
            < ok.

            ( 0x10, 0x32, 0x54, 0x76 )
            > reader
            < 16 50 84 118 ok.

            (                 |------------|                    x = ptrword[i] )
            (                                |-|                x += i         )
            (                                    | -----------| ptrword[i] = x )
            > : writer 4 0 do i ptrword + b@ i + ptrword i + b! loop ;
            < ok.
            > writer
            < ok.
            ( 0x10, 0x33, 0x56, 0x79 )
            > reader
            < 16 51 86 121 ok.
        "#);
    }

    #[test]
    fn execute() {
        all_runtest(r#"
            ( define two words )
            > : hello ." hello, world!" ;
            < ok.
            > : goodbye ." goodbye, world!" ;
            < ok.

            ( take their addresses )
            > ' goodbye
            < ok.
            > ' hello
            < ok.

            ( and exec them! )
            > execute
            < hello, world!ok.
            > execute
            < goodbye, world!ok.
        "#);
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
                    let word = Word::data(self.ctr as i32);
                    self.forth.data_stack.push(word)?;
                    self.ctr += 1;
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
        use crate::{dictionary::{AsyncBuiltins, AsyncBuiltinEntry}, fastr::FaStr, async_builtin, testutil::async_blockon_runtest_with_dispatcher};

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

        async_blockon_runtest_with_dispatcher(
            TestContext::default(),
            TestAsyncDispatcher, r#"
                ( stack is empty... )
                x .

                ( async builtin... )
                > 5 counter
                < ok.

                ( exactly 5 placed back on the stack )
                > .
                < 5 ok.
                x .
            "#
        );
    }

    #[test]
    fn compile() {
        all_runtest(r#"
            > 2 3 + .
            < 5 ok.
            > : yay 2 3 + . ;
            < ok.
            > yay yay yay
            < 5 5 5 ok.
            > : boop yay yay ;
            < ok.
            > boop
            x 5 5 ok.
            > : err if boop boop boop else yay yay then ;
            < ok.
            > : erf if boop boop boop then yay yay ;
            < ok.
            > 0 err
            < 5 5 ok.
            > 1 err
            < 5 5 5 5 5 5 ok.
            > 0 erf
            < 5 5 ok.
            > 1 erf
            < 5 5 5 5 5 5 5 5 ok.
        "#);
    }

    #[test]
    fn nested_if_else() {
        all_runtest(r#"
            > : one 1 . ;
            < ok.
            > : two 2 . ;
            < ok.
            > : six 6 . ;
            < ok.
            > : nif if one if two two else six then one then ;
            < ok.
            >   0 nif
            < ok.
            > 0 1 nif
            < 1 6 1 ok.
            > 1 1 nif
            < 1 2 2 1 ok.
        "#);
    }

    #[test]
    fn do_loop() {
        all_runtest(r#"
            > : one 1 . ;
            < ok.
            > : six 6 . ;
            < ok.
            > 42 emit
            < *ok.
            > : star 42 emit ;
            < ok.
            > star star star
            < ***ok.
            > : sloop one 5 0 do star star loop six ;
            < ok.
            > sloop
            < 1 **********6 ok.
            > : count 10 0 do i . loop ;
            < ok.
            > count
            < 0 1 2 3 4 5 6 7 8 9 ok.
            > : smod 10 0 do i 3 mod not if star then loop ;
            < ok.
            > smod
            < ****ok.
        "#);
    }

    #[test]
    fn strings() {
        all_runtest(r#"
            > : beep ." hello, world!" ;
            < ok.
            > beep
            < hello, world!ok.
        "#);
    }

    #[test]
    fn constants() {
        all_runtest(r#"
            > constant x 123
            < ok.
            > x .
            < 123 ok.
            > 4 x + .
            < 127 ok.
        "#);
    }

    #[test]
    fn variables_and_arrays() {
        all_runtest(r#"
            > variable y
            < ok.
            > y @ .
            < 0 ok.
            > 10 y !
            < ok.
            > y @ .
            < 10 ok.
            > array z 4
            < ok.
            > z @ . z 1 w+ @ . z 2 w+ @ . z 3 w+ @ .
            < 0 0 0 0 ok.
            > 10 z ! 20 z 1 w+ ! 30 z 2 w+ ! 40 z 3 w+ !
            < ok.
            > z @ . z 1 w+ @ . z 2 w+ @ . z 3 w+ @ .
            < 10 20 30 40 ok.
            > forget z
            < ok.
            > variable a
            < ok.
            > 100 a !
            < ok.
            > array z 4
            < ok.
            > z @ . z 1 w+ @ . z 2 w+ @ . z 3 w+ @ .
            < 0 0 0 0 ok.
        "#);
    }
}
