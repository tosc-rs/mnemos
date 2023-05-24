use core::{fmt::Write, mem::size_of, ptr::NonNull, marker::PhantomData};

use crate::{
    dictionary::{BuiltinEntry, DictionaryEntry, EntryHeader, EntryKind, DictLocation},
    fastr::comptime_fastr,
    vm::TmpFaStr,
    word::Word,
    Error, Forth, Mode, ReplaceErr, Lookup,
};

#[cfg(feature = "floats")]
pub mod floats;

// NOTE: This macro exists because we can't have const constructors that include
// "mut" items, which unfortunately covers things like `fn(&mut T)`. Use a macro
// until this is resolved.
#[macro_export]
macro_rules! builtin {
    ($name:literal, $func:expr) => {
        BuiltinEntry {
            hdr: EntryHeader {
                name: comptime_fastr($name),
                kind: EntryKind::StaticBuiltin,
                len: 0,
                _pd: core::marker::PhantomData,
            },
            func: $func,
        }
    };
}

/// Constructs an [`AsyncBuiltinEntry`](crate::dictionary::AsyncBuiltinEntry)
/// for an asynchronous builtin word.
///
/// See the [documentation for `AsyncForth`](crate::AsyncForth) for details on
/// using asynchronous builtin words.
#[macro_export]
macro_rules! async_builtin {
    ($name:literal) => {
        $crate::dictionary::AsyncBuiltinEntry {
            hdr: $crate::dictionary::EntryHeader {
                name: $crate::fastr::comptime_fastr($name),
                kind: $crate::dictionary::EntryKind::AsyncBuiltin,
                len: 0,
                _pd: core::marker::PhantomData,
            },
        }
    };
}

#[macro_export]
macro_rules! builtin_if_feature {
    ($feature:literal, $name:literal, $func:expr) => {
        #[cfg(feature = $feature)]
        builtin!($name, $func)
    };
}

// let literal_dict = self.find_word("(literal)").ok_or(Error::WordNotInDict)?;

impl<T: 'static> Forth<T> {
    pub const FULL_BUILTINS: &'static [BuiltinEntry<T>] = &[
        //
        // Math operations
        //
        builtin!("+", Self::add),
        builtin!("-", Self::minus),
        builtin!("/", Self::div),
        builtin!("mod", Self::modu),
        builtin!("/mod", Self::div_mod),
        builtin!("*", Self::mul),
        builtin!("abs", Self::abs),
        builtin!("negate", Self::negate),
        builtin!("min", Self::min),
        builtin!("max", Self::max),
        //
        // Floating Math operations
        //
        builtin_if_feature!("floats", "f+", Self::float_add),
        builtin_if_feature!("floats", "f-", Self::float_minus),
        builtin_if_feature!("floats", "f/", Self::float_div),
        builtin_if_feature!("floats", "fmod", Self::float_modu),
        builtin_if_feature!("floats", "f/mod", Self::float_div_mod),
        builtin_if_feature!("floats", "f*", Self::float_mul),
        builtin_if_feature!("floats", "fabs", Self::float_abs),
        builtin_if_feature!("floats", "fnegate", Self::float_negate),
        builtin_if_feature!("floats", "fmin", Self::float_min),
        builtin_if_feature!("floats", "fmax", Self::float_max),
        //
        // Double intermediate math operations
        //
        builtin!("*/", Self::star_slash),
        builtin!("*/mod", Self::star_slash_mod),
        //
        // Logic operations
        //
        builtin!("not", Self::invert),
        // NOTE! This is `bitand`, not logical `and`! e.g. `&` not `&&`.
        builtin!("and", Self::and),
        builtin!("=", Self::equal),
        builtin!(">", Self::greater),
        builtin!("<", Self::less),
        builtin!("0=", Self::zero_equal),
        builtin!("0>", Self::zero_greater),
        builtin!("0<", Self::zero_less),
        //
        // Stack operations
        //
        builtin!("swap", Self::swap),
        builtin!("dup", Self::dup),
        builtin!("over", Self::over),
        builtin!("rot", Self::rot),
        builtin!("drop", Self::ds_drop),
        //
        // Double operations
        //
        builtin!("2swap", Self::swap_2),
        builtin!("2dup", Self::dup_2),
        builtin!("2over", Self::over_2),
        builtin!("2drop", Self::ds_drop_2),
        //
        // String/Output operations
        //
        builtin!("emit", Self::emit),
        builtin!("cr", Self::cr),
        builtin!("space", Self::space),
        builtin!("spaces", Self::spaces),
        builtin!(".", Self::pop_print),
        builtin!("u.", Self::unsigned_pop_print),
        builtin_if_feature!("floats", "f.", Self::float_pop_print),
        //
        // Define/forget
        //
        builtin!(":", Self::colon),
        builtin!("forget", Self::forget),
        //
        // Stack/Retstack operations
        //
        builtin!("d>r", Self::data_to_return_stack),
        // NOTE: REQUIRED for `do/loop`
        builtin!("2d>2r", Self::data2_to_return2_stack),
        builtin!("r>d", Self::return_to_data_stack),
        //
        // Loop operations
        //
        builtin!("i", Self::loop_i),
        builtin!("i'", Self::loop_itick),
        builtin!("j", Self::loop_j),
        builtin!("leave", Self::loop_leave),
        //
        // Memory operations
        //
        builtin!("@", Self::var_load),
        builtin!("!", Self::var_store),
        builtin!("b@", Self::byte_var_load),
        builtin!("b!", Self::byte_var_store),
        builtin!("w+", Self::word_add),
        builtin!("'", Self::addr_of),
        builtin!("execute", Self::execute),
        //
        // Constants
        //
        builtin!("0", Self::zero_const),
        builtin!("1", Self::one_const),
        //
        // Introspection
        //
        builtin!("builtins", Self::list_builtins),
        builtin!("dict", Self::list_dict),
        builtin!(".s", Self::list_stack),
        builtin!("free", Self::dict_free),
        //
        // Other
        //
        // NOTE: REQUIRED for `."`
        builtin!("(write-str)", Self::write_str_lit),
        // NOTE: REQUIRED for `do/loop`
        builtin!("(jmp-doloop)", Self::jump_doloop),
        // NOTE: REQUIRED for `if/then` and `if/else/then`
        builtin!("(jump-zero)", Self::jump_if_zero),
        // NOTE: REQUIRED for `if/else/then`
        builtin!("(jmp)", Self::jump),
        // NOTE: REQUIRED for `:` (if you want literals)
        builtin!("(literal)", Self::literal),
        // NOTE: REQUIRED for `constant`
        builtin!("(constant)", Self::constant),
        // NOTE: REQUIRED for `variable` or `array`
        builtin!("(variable)", Self::variable),
    ];

    pub fn dict_free(&mut self) -> Result<(), Error> {
        let capa = self.dict.alloc.capacity();
        let used = self.dict.alloc.used();
        let free = capa - used;
        writeln!(
            &mut self.output,
            "{}/{} bytes free ({} used)",
            free, capa, used
        )?;
        Ok(())
    }

    pub fn list_stack(&mut self) -> Result<(), Error> {
        let depth = self.data_stack.depth();
        write!(&mut self.output, "<{}> ", depth)?;
        for d in (0..depth).rev() {
            let val = self.data_stack.try_peek_back_n(d)?;
            write!(&mut self.output, "{} ", unsafe { val.data })?;
        }
        self.output.push_str("\n")?;
        Ok(())
    }

    pub fn list_builtins(&mut self) -> Result<(), Error> {
        let Self {
            builtins, output, ..
        } = self;
        output.write_str("builtins: ")?;
        for bi in builtins.iter() {
            output.write_str(bi.hdr.name.as_str())?;
            output.write_str(", ")?;
        }
        output.write_str("\n")?;
        Ok(())
    }

    pub fn list_dict(&mut self) -> Result<(), Error> {
        let Self { output, dict, .. } = self;
        output.write_str("dictionary: ")?;
        for item in dict.entries() {
            output.write_str(unsafe { item.entry().as_ref() }.hdr.name.as_str())?;
            if let DictLocation::Parent(_) = item {
                // indicate that this binding is inherited from a parent.
                // XXX(eliza): i was initially gonna add "(inherited)" but that
                // makes some of the tests overflow their output buffer, lol.
                output.write_str("*")?;
            }
            output.write_str(", ")?;
        }
        output.write_str("\n")?;
        Ok(())
    }

    // addr offset w+
    pub fn word_add(&mut self) -> Result<(), Error> {
        let w_offset = self.data_stack.try_pop()?;
        let w_addr = self.data_stack.try_pop()?;
        let new_addr = unsafe {
            let offset = isize::try_from(w_offset.data).replace_err(Error::BadWordOffset)?;
            w_addr.ptr.cast::<Word>().offset(offset)
        };
        self.data_stack.push(Word::ptr(new_addr))?;
        Ok(())
    }

    pub fn byte_var_load(&mut self) -> Result<(), Error> {
        let w = self.data_stack.try_pop()?;
        let ptr = unsafe { w.ptr.cast::<u8>() };
        let val = unsafe { Word::data(i32::from(ptr.read())) };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn byte_var_store(&mut self) -> Result<(), Error> {
        let w_addr = self.data_stack.try_pop()?;
        let w_val = self.data_stack.try_pop()?;
        unsafe {
            w_addr.ptr.cast::<u8>().write((w_val.data & 0xFF) as u8);
        }
        Ok(())
    }

    // TODO: Check alignment?
    pub fn var_load(&mut self) -> Result<(), Error> {
        let w = self.data_stack.try_pop()?;
        let ptr = unsafe { w.ptr.cast::<Word>() };
        let val = unsafe { ptr.read() };
        self.data_stack.push(val)?;
        Ok(())
    }

    // TODO: Check alignment?
    pub fn var_store(&mut self) -> Result<(), Error> {
        let w_addr = self.data_stack.try_pop()?;
        let w_val = self.data_stack.try_pop()?;
        unsafe {
            w_addr.ptr.cast::<Word>().write(w_val);
        }
        Ok(())
    }

    pub fn zero_const(&mut self) -> Result<(), Error> {
        self.data_stack.push(Word::data(0))?;
        Ok(())
    }

    pub fn one_const(&mut self) -> Result<(), Error> {
        self.data_stack.push(Word::data(1))?;
        Ok(())
    }

    pub fn constant(&mut self) -> Result<(), Error> {
        let me = self.call_stack.try_peek()?;
        let de = me.eh.cast::<DictionaryEntry<T>>();
        let cfa = unsafe { DictionaryEntry::<T>::pfa(de) };
        let val = unsafe { cfa.as_ptr().read() };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn variable(&mut self) -> Result<(), Error> {
        let me = self.call_stack.try_peek()?;
        let de = me.eh.cast::<DictionaryEntry<T>>();
        let cfa = unsafe { DictionaryEntry::<T>::pfa(de) };
        let val = Word::ptr(cfa.as_ptr());
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn forget(&mut self) -> Result<(), Error> {
        // TODO: If anything we've defined in the dict has escaped into
        // the stack, variables, etc., we're definitely going to be in trouble.
        //
        // TODO: Check that we are in interpret and not compile mode?
        self.input.advance();
        let word = match self.input.cur_word() {
            None => return Err(Error::ForgetWithoutWordName),
            Some(s) => s,
        };
        let word_tmp = TmpFaStr::new_from(word);
        let defn = match self.find_in_dict(&word_tmp) {
            None => {
                if self.find_in_bis(&word_tmp).is_some() {
                    return Err(Error::CantForgetBuiltins);
                } else {
                    return Err(Error::ForgetNotInDict);
                }
            }
            Some(d) => d,
        };

        match defn {
            // The definition is in the current (mutable) dictionary. We can
            // forget it by zeroing out the entry in the current dictionary.
            DictLocation::Current(defn) => {
                // NOTE: We use the *name* pointer for rewinding, as we allocate the name before the item.
                let name_ptr = unsafe { defn.as_ref().hdr.name.as_ptr().cast_mut() };
                self.dict.tail = unsafe { defn.as_ref().link };
                let addr = defn.as_ptr();
                let name_contains = self.dict.alloc.contains(name_ptr.cast());
                let contains = self.dict.alloc.contains(addr.cast());
                let ordered = (addr as usize) <= (self.dict.alloc.cur as usize);

                if !(name_contains && contains && ordered) {
                    return Err(Error::InternalError);
                }

                let len = (self.dict.alloc.cur as usize) - (name_ptr as usize);
                unsafe {
                    name_ptr.write_bytes(0x00, len);
                }
                self.dict.alloc.cur = name_ptr;
            },
            // The definition is in a parent (frozen) dictionary. We can't
            // mutate that dictionary, so we must create a new entry in the
            // current dict saying that the definition is forgotten.
            // XXX(eliza): or this could be a runtime error? IDK...
            DictLocation::Parent(_de) => {
                todo!("eliza: forget parent definitions");
            }
        }

        Ok(())
    }

    pub fn over(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_peek_back_n(1)?;
        self.data_stack.push(a)?;
        Ok(())
    }

    pub fn over_2(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_peek_back_n(2)?;
        let b = self.data_stack.try_peek_back_n(3)?;
        self.data_stack.push(b)?;
        self.data_stack.push(a)?;
        Ok(())
    }

    pub fn rot(&mut self) -> Result<(), Error> {
        let n1 = self.data_stack.try_pop()?;
        let n2 = self.data_stack.try_pop()?;
        let n3 = self.data_stack.try_pop()?;
        self.data_stack.push(n2)?;
        self.data_stack.push(n1)?;
        self.data_stack.push(n3)?;
        Ok(())
    }

    pub fn ds_drop(&mut self) -> Result<(), Error> {
        let _a = self.data_stack.try_pop()?;
        Ok(())
    }

    pub fn ds_drop_2(&mut self) -> Result<(), Error> {
        let _a = self.data_stack.try_pop()?;
        let _b = self.data_stack.try_pop()?;
        Ok(())
    }

    pub fn swap(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack.push(a)?;
        self.data_stack.push(b)?;
        Ok(())
    }

    pub fn swap_2(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let c = self.data_stack.try_pop()?;
        let d = self.data_stack.try_pop()?;
        self.data_stack.push(b)?;
        self.data_stack.push(a)?;
        self.data_stack.push(d)?;
        self.data_stack.push(c)?;
        Ok(())
    }

    pub fn space(&mut self) -> Result<(), Error> {
        self.output.push_bstr(b" ")?;
        Ok(())
    }

    pub fn spaces(&mut self) -> Result<(), Error> {
        let num = self.data_stack.try_pop()?;
        let num = unsafe { num.data };

        if num.is_negative() {
            return Err(Error::LoopCountIsNegative);
        }
        for _ in 0..num {
            self.space()?;
        }
        Ok(())
    }

    pub fn cr(&mut self) -> Result<(), Error> {
        self.output.push_bstr(b"\n")?;
        Ok(())
    }

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

    pub fn and(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = Word::data(unsafe { a.data & b.data });
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

    pub fn greater(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = if unsafe { b.data > a.data } { -1 } else { 0 };
        self.data_stack.push(Word::data(val))?;
        Ok(())
    }

    pub fn less(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = if unsafe { b.data < a.data } { -1 } else { 0 };
        self.data_stack.push(Word::data(val))?;
        Ok(())
    }

    pub fn zero_equal(&mut self) -> Result<(), Error> {
        self.data_stack.push(Word::data(0))?;
        self.equal()
    }

    pub fn zero_greater(&mut self) -> Result<(), Error> {
        self.data_stack.push(Word::data(0))?;
        self.greater()
    }

    pub fn zero_less(&mut self) -> Result<(), Error> {
        self.data_stack.push(Word::data(0))?;
        self.less()
    }

    pub fn div_mod(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        if unsafe { a.data == 0 } {
            return Err(Error::DivideByZero);
        }
        let rem = unsafe { Word::data(b.data % a.data) };
        self.data_stack.push(rem)?;
        let val = unsafe { Word::data(b.data / a.data) };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn div(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = unsafe {
            if a.data == 0 {
                return Err(Error::DivideByZero);
            }
            Word::data(b.data / a.data)
        };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn modu(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        let val = unsafe {
            if a.data == 0 {
                return Err(Error::DivideByZero);
            }
            Word::data(b.data % a.data)
        };
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn loop_i(&mut self) -> Result<(), Error> {
        let a = self.return_stack.try_peek()?;
        self.data_stack.push(a)?;
        Ok(())
    }

    pub fn loop_itick(&mut self) -> Result<(), Error> {
        let a = self.return_stack.try_peek_back_n(1)?;
        self.data_stack.push(a)?;
        Ok(())
    }

    pub fn loop_j(&mut self) -> Result<(), Error> {
        let a = self.return_stack.try_peek_back_n(2)?;
        self.data_stack.push(a)?;
        Ok(())
    }

    pub fn loop_leave(&mut self) -> Result<(), Error> {
        let _ = self.return_stack.try_pop()?;
        let a = self.return_stack.try_peek()?;
        self.return_stack
            .push(unsafe { Word::data(a.data.wrapping_sub(1)) })?;
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
        let offset = parent.get_current_val()?;
        parent.offset(offset)?;
        Ok(())
    }

    pub fn dup(&mut self) -> Result<(), Error> {
        let val = self.data_stack.try_peek()?;
        self.data_stack.push(val)?;
        Ok(())
    }

    pub fn dup_2(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack.push(b)?;
        self.data_stack.push(a)?;
        self.data_stack.push(b)?;
        self.data_stack.push(a)?;
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
        write!(&mut self.output, "{} ", unsafe { a.data })?;
        Ok(())
    }

    pub fn unsigned_pop_print(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        write!(&mut self.output, "{} ", unsafe { a.data } as u32)?;
        Ok(())
    }

    /// # Add (`+`)
    ///
    /// ```rust
    /// # use forth3::testutil::blocking_runtest;
    /// #
    /// # blocking_runtest(r#"
    /// > 1 2 +
    /// > .
    /// < 3 ok.
    /// # "#)
    pub fn add(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;

        // NOTE: CURSED BECAUSE OF POINTER MATH
        // context: https://cohost.org/jamesmunns/post/851945-oops-it-segfaults
        self.data_stack
            .push(Word::ptr_data(unsafe {
                let a = a.ptr as isize;
                let b = b.ptr as isize;
                a.wrapping_add(b)
            }))?;
        Ok(())
    }

    pub fn mul(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::data(unsafe { a.data.wrapping_mul(b.data) }))?;
        Ok(())
    }

    pub fn abs(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::data(unsafe { a.data.wrapping_abs() }))?;
        Ok(())
    }

    pub fn negate(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::data(unsafe { a.data.wrapping_neg() }))?;
        Ok(())
    }

    pub fn min(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::data(unsafe { a.data.min(b.data) }))?;
        Ok(())
    }

    pub fn max(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        self.data_stack
            .push(Word::data(unsafe { a.data.max(b.data) }))?;
        Ok(())
    }

    pub fn minus(&mut self) -> Result<(), Error> {
        let a = self.data_stack.try_pop()?;
        let b = self.data_stack.try_pop()?;
        // NOTE: CURSED BECAUSE OF POINTER MATH
        // context: https://cohost.org/jamesmunns/post/851945-oops-it-segfaults
        self.data_stack
            .push(Word::ptr_data(unsafe {
                let a = a.ptr as isize;
                let b = b.ptr as isize;
                b.wrapping_sub(a)
            }))?;
        Ok(())
    }

    pub fn star_slash(&mut self) -> Result<(), Error> {
        let n3 = self.data_stack.try_pop()?;
        let n2 = self.data_stack.try_pop()?;
        let n1 = self.data_stack.try_pop()?;
        self.data_stack.push(Word::data(unsafe {
            (n1.data as i64)
                .wrapping_mul(n2.data as i64)
                .wrapping_div(n3.data as i64) as i32
        }))?;
        Ok(())
    }

    pub fn star_slash_mod(&mut self) -> Result<(), Error> {
        let n3 = self.data_stack.try_pop()?;
        let n2 = self.data_stack.try_pop()?;
        let n1 = self.data_stack.try_pop()?;
        unsafe {
            let top = (n1.data as i64).wrapping_mul(n2.data as i64);
            let div = n3.data as i64;
            let quo = top / div;
            let rem = top % div;
            self.data_stack.push(Word::data(rem as i32))?;
            self.data_stack.push(Word::data(quo as i32))?;
        }
        Ok(())
    }

    pub fn colon(&mut self) -> Result<(), Error> {
        let old_mode = core::mem::replace(&mut self.mode, Mode::Compile);
        let name = self.munch_name()?;

        // Allocate and initialize the dictionary entry
        //
        // TODO: Using `bump_write` here instead of just `bump` causes Miri to
        // get angry with a stacked borrows violation later when we attempt
        // to interpret a built word.
        // TODO(eliza): it's unfortunate we cannot easily use the "EntryBuilder"
        // type here, as it must mutably borrow the dictionary, and `munch_one`
        // must perform lookups...hmm...
        let dict_base = self.dict.alloc.bump::<DictionaryEntry<T>>()?;

        let mut len = 0u16;

        // Begin compiling until we hit the end of the line or a semicolon.
        loop {
            let munched = self.munch_one(&mut len)?;
            if munched == 0 {
                match self.input.cur_word() {
                    Some(";") => {
                        unsafe {
                            dict_base.as_ptr().write(DictionaryEntry {
                                hdr: EntryHeader {
                                    name,
                                    kind: EntryKind::Dictionary,
                                    len,
                                    _pd: PhantomData,
                                },
                                // TODO: Should we look up `(interpret)` for consistency?
                                // Use `find_word`?
                                func: Self::interpret,
                                // Don't link until we know we have a "good" entry!
                                link: self.dict.tail.take(),
                                parameter_field: [],
                            });
                        }
                        self.dict.tail = Some(dict_base);
                        self.mode = old_mode;
                        return Ok(());
                    }
                    Some(_) => {}
                    None => {
                        return Err(Error::ColonCompileMissingSemicolon);
                    }
                }
            }
        }
    }

    pub fn write_str_lit(&mut self) -> Result<(), Error> {
        let parent = self.call_stack.try_peek_back_n_mut(1)?;

        // The length in bytes is stored in the next word.
        let len = parent.get_current_val()?;
        let len_u16 = u16::try_from(len).replace_err(Error::LiteralStringTooLong)?;

        // Now we need to figure out how many words our inline string takes up
        let word_size = size_of::<Word>();
        let len_words = 1 + ((usize::from(len_u16) + (word_size - 1)) / word_size);
        let len_and_str = parent.get_next_n_words(len_words as u16)?;
        unsafe {
            // Skip the "len" word
            let start = len_and_str.as_ptr().add(1).cast::<u8>();
            // Then push the literal into the output buffer
            let u8_sli = core::slice::from_raw_parts(start, len_u16.into());
            self.output.push_bstr(u8_sli)?;
        }
        parent.offset(len_words as i32)?;
        Ok(())
    }

    /// `(literal)` is used mid-interpret to put the NEXT word of the parent's
    /// CFA array into the stack as a value.
    pub fn literal(&mut self) -> Result<(), Error> {
        let parent = self.call_stack.try_peek_back_n_mut(1)?;
        let literal = parent.get_current_word()?;
        parent.offset(1)?;
        self.data_stack.push(literal)?;
        Ok(())
    }

    /// Looks up a name in the dictionary and places its address on the stack.
    pub fn addr_of(&mut self) -> Result<(), Error> {
        self.input.advance();
        let name = self
            .input
            .cur_word()
            .ok_or(Error::AddrOfMissingName)?;
        match self.lookup(name)? {
            // The definition is in the current dictionary --- just push it.
            Lookup::Dict(DictLocation::Current(de)) =>
                self.data_stack.push(Word::ptr(de.as_ptr()))?,

            // The definition is in the parent (frozen) dictionary.
            // TODO(eliza): what should we do here?
            Lookup::Dict(DictLocation::Parent(de)) =>
                self.data_stack.push(Word::ptr(de.as_ptr()))?,

            Lookup::Builtin { bi } =>
                self.data_stack.push(Word::ptr(bi.as_ptr()))?,

            #[cfg(feature = "async")]
            Lookup::Async { bi } =>
                self.data_stack.push(Word::ptr(bi.as_ptr()))?,
            _ => return Err(Error::AddrOfNotAWord),
        }

        Ok(())
    }

    pub fn execute(&mut self) -> Result<(), Error> {
        let w = self.data_stack.try_pop()?;
        // pop the execute word off the stack
        self.call_stack.pop();
        unsafe {
            // Safety: YOLO :D
            let eh = w.ptr.cast::<EntryHeader<T>>();
            self.call_stack.push(crate::vm::CallContext {
                eh: NonNull::new_unchecked(eh),
                len: (*eh).len,
                idx: 0,
            })?;
        };

        Err(Error::PendingCallAgain)
    }
}
