use core::{mem::transmute, ptr::NonNull, str::FromStr};

pub mod cfa;
pub mod dictionary;
pub mod input;
pub mod name;
pub mod stack;
pub mod word;

use crate::{
    dictionary::{DictionaryBump, DictionaryEntry},
    input::WordStrBuf,
    name::{Mode, Name},
    stack::Stack,
    word::Word,
};

/// `WordFunc` represents a function that can be used as part of a dictionary word.
///
/// It takes the current "full context" (e.g. `Fif`), as well as the CFA pointer
/// to the dictionary entry.
type WordFunc<'a, 'b> = fn(Fif<'a, 'b>, *mut Word) -> Result<(), ()>;

/// `BuildinFunc` represents a function that is recognized as a pre-compiled built-in
/// of the VM/interpreter. It does not take a CFA, as it is not stored as a dictionary
/// entry.
type BuiltinFunc<'a, 'b> = fn(Fif<'a, 'b>) -> Result<(), ()>;

/// Forth is the "context" of the VM/interpreter.
///
/// It does NOT include the input/output buffers, or any components that
/// directly rely on those buffers. This Forth context is composed with
/// the I/O buffers to create the `Fif` type. This is done for lifetime
/// reasons.
pub struct Forth {
    mode: Mode,
    data_stack: Stack,
    dict_alloc: DictionaryBump,
    run_dict_tail: Option<NonNull<DictionaryEntry>>,

    // TODO: This will be for words that have compile time actions, I guess?
    _comp_dict_tail: Option<NonNull<DictionaryEntry>>,
}

impl Forth {
    pub unsafe fn new(stack_buf: (*mut Word, usize), dict_buf: (*mut u8, usize)) -> Self {
        let data_stack = Stack::new(stack_buf.0, stack_buf.1);
        let dict_alloc = DictionaryBump::new(dict_buf.0, dict_buf.1);
        Self {
            mode: Mode::Run,
            data_stack,
            dict_alloc,
            run_dict_tail: None,
            _comp_dict_tail: None,
        }
    }

    fn parse_num(word: &str) -> Option<i32> {
        i32::from_str(word).ok()
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

    fn find_builtin<'a, 'b>(word: &'b str) -> Option<BuiltinFunc<'a, 'b>> {
        Fif::BUILTINS.iter().find_map(|(n, func)| {
            if *n == word {
                let func: BuiltinFunc<'static, 'static> = *func;
                let func: BuiltinFunc<'a, 'b> = unsafe { core::mem::transmute(func) };
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
                    let before_compile_alloc = self.dict_alloc.cur;
                    let before_compile_dict = self.run_dict_tail.clone();
                    // TODO: Also check run dict, once we use that
                    assert!(self._comp_dict_tail.is_none());

                    let res = func(Fif {
                        forth: self,
                        input: line,
                    });

                    // If we are compiling, and the process fails, rewind the allocator
                    // to this position
                    if func == Fif::colon {
                        if res.is_err() {
                            if before_compile_dict == self.run_dict_tail {
                                // Rewind the allocator to before the start of this compilation
                                self.dict_alloc.cur = before_compile_alloc;
                            } else {
                                #[cfg(test)]
                                panic!("compilation failed but dict LL was modified?");
                            }
                        }
                    }

                    res
                }
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
    const BUILTINS: &'static [(&'static str, BuiltinFunc<'static, 'static>)] =
        &[("add", Fif::add), (".", Fif::pop_print), (":", Fif::colon)];

    pub fn pop_print(self) -> Result<(), ()> {
        let _a = self.forth.data_stack.pop().ok_or(())?;
        #[cfg(test)]
        print!("{} ", unsafe { _a.data });
        Ok(())
    }

    pub fn add(self) -> Result<(), ()> {
        let a = self.forth.data_stack.pop().ok_or(())?;
        let b = self.forth.data_stack.pop().ok_or(())?;
        self.forth
            .data_stack
            .push(Word::data(unsafe { a.data.wrapping_add(b.data) }))
    }

    pub fn colon(self) -> Result<(), ()> {
        let name = self.input.next_word().ok_or(())?;
        if Fif::BUILTINS.iter().map(|(name, _func)| name).any(|bin| *bin == name) {
            return Err(());
        }

        let old_mode = core::mem::replace(&mut self.forth.mode, Mode::Compile);
        let name = Name::new_from_bstr(Mode::Run, name.as_bytes());

        // Allocate and initialize the dictionary entry
        //
        // TODO: Using `bump_write` here instead of just `bump` causes Miri to
        // get angry with a stacked borrows violation later when we attempt
        // to interpret a built word.
        let mut dict_base = self.forth.dict_alloc.bump::<DictionaryEntry>().ok_or(())?;
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
            let len_word = self.forth.dict_alloc.bump::<Word>().ok_or(())?;
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
                Lookup::Builtin { func } => {
                    // Builtins are put into the CFA array directly as a pointer
                    // to the builtin function.
                    let fptr: *mut () = func as *mut ();
                    self.forth.dict_alloc.bump_write(Word::ptr(fptr))?;
                    *len += 1;
                }
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
                    // 1. The address of the `literal()` function, used as a sentinel
                    // 2. The value of the literal, as a data word
                    let fptr: *mut () = Fif::literal as *mut ();
                    self.forth.dict_alloc.bump_write(Word::ptr(fptr))?;
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
            Err(())
        }
    }

    /// Literal is generally only used as a sentinel value in compiled
    /// words to note that the next item in the CFA is a `u32`.
    ///
    /// It generally shouldn't ever be called.
    pub fn literal(self) -> Result<(), ()> {
        #[cfg(test)]
        panic!();
        #[allow(unreachable_code)]
        Err(())
    }

    /// Interpret is the run-time target of the `:` (colon) word.
    ///
    /// It is NOT considered a "builtin", as it DOES take the cfa, where
    /// other builtins do not.
    pub fn interpret(self, cfa: *mut Word) -> Result<(), ()> {
        // The "literal" built-in word is used as a sentinel. See below.
        const LIT: *mut () = Fif::literal as *mut ();

        // Colon compiles into a list of words, where the first word
        // is a `u32` of the `len` number of words.
        let mut words = unsafe {
            let len = *cfa.cast::<u32>() as usize;
            if len == 0 {
                return Ok(());
            }
            // Skip the "len" field, which is the first word
            // of the cfa.
            core::slice::from_raw_parts(cfa.add(1), len)
        }
        .iter();

        // For the remaining words, we do a while-let loop instead of
        // a for-loop, as some words (e.g. literals) require advancing
        // to the next word.
        while let Some(word) = words.next() {
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
                wf(fif2, cfa.as_ptr())?;
            } else if LIT == ptr {
                // If this word is SPECIFICALLY the `literal` builtin, then we
                // need to treat the NEXT word as a u32, and push it on the
                // stack.
                let lit = words.next().ok_or(())?;
                let val = unsafe { lit.data };
                fif2.forth.data_stack.push(Word::data(val))?;
            } else {
                // The
                let builtin = Self::BUILTINS.iter().find_map(|(_name, func)| {
                    let bif = (*func) as *mut ();
                    if bif == ptr {
                        let a: BuiltinFunc<'static, 'static> = *func;
                        let b: BuiltinFunc<'_, '_> = unsafe { transmute(a) };
                        Some(b)
                    } else {
                        None
                    }
                });

                builtin.ok_or(())?(fif2)?;
            }
        }

        Ok(())
    }
}

pub enum Lookup<'a, 'b> {
    Builtin { func: BuiltinFunc<'a, 'b> },
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
        let payload_stack: LeakBox<Word, 256> = LeakBox::new();
        let input_buf: LeakBox<u8, 256> = LeakBox::new();
        let dict_buf: LeakBox<u8, 512> = LeakBox::new();

        let mut input = WordStrBuf::new(input_buf.ptr(), input_buf.len());
        let mut forth = unsafe {
            Forth::new(
                (payload_stack.ptr(), payload_stack.len()),
                (dict_buf.ptr(), dict_buf.len()),
            )
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
