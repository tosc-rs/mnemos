pub struct Something;
pub struct SomethingUp;
pub struct SomethingDown;
pub struct SomethingVariablySized;

// Use a union so that things work on both 32- and 64-bit systems,
// so the *data* is always 32 bits, but the pointer is whatever the
// native word size is.
pub union Word {
    data: u32,
    ptr: *mut (),
}

impl Word {
    #[inline]
    fn data(data: u32) -> Self {
        Word { data }
    }

    #[inline]
    fn ptr<T>(ptr: *mut T) -> Self {
        Word { ptr: ptr.cast() }
    }
}

// Starting FORTH: page 231
pub struct Everything {
    /// Precompiled forth words
    builtin_words: Something,
    /// Variables that affect the system
    system_variables: Something,
    /// Option (also compiled?) forth words
    elective_definitions: Something,
    /// User dictionary
    user_dictionary: SomethingUp,
    // /// hmm
    // pad: Something, // technically this lives at the top of the user dict?
    /// Main stack
    parameter_stack: Stack,
    /// Input scratch buffer
    input_msg_buffer: SomethingUp,
    /// Return (secondary) stack
    return_stack: Stack,
    /// User variable heapish thing
    user_variables: Something,
    /// Used for paging from disk
    block_buffers: Something,
}

// Starting FORTH: page 220
pub struct DictionaryEntry {
    /// Precedence bit, length, and text characters
    /// Precedence bit is used to determine if it runs at compile or run time
    name: Something,
    /// Link field, points back to the previous entry
    link: Something,

    // HEAD ^
    // ------
    // BODY v

    /// Next is the "code pointer." The address contained in this
    /// pointer is what distinguishes a variable from a constant or a
    /// colon definition. It is the address of the instruction that is
    /// executed first when the particular type of word is executed.
    /// For example, in the case of a variable, the pointer points to code
    /// that pushes the address of the variable onto the stack.
    ///
    /// In the case of a constant, the pointer points to code that pushes the
    /// contents of the constant onto the stack. In the case of a colon
    /// definition, the pointer points to code that executes the rest of
    /// the words in the colon definition.
    ///
    /// The code that is pointed to is called the "run-time code"
    /// because it's used when a word of that type is executed (not when
    /// a word of that type is defined or compiled).
    code_pointer: Something,
    /// data OR an array of compiled code.
    /// the first word is the "p(arameter)fa" or "c(ode)fa"
    parameter_field: SomethingVariablySized,
}


pub struct Stack {
    top: *mut Word,
    cur: *mut Word,
    bot: *mut Word,
}

impl Stack {
    pub fn new(bottom: *mut Word, words: usize) -> Self {
        let top = bottom.wrapping_add(words);
        debug_assert!(top >= bottom);
        Self {
            top,
            bot: bottom,
            cur: top,
        }
    }

    #[inline]
    pub fn push(&mut self, word: Word) -> Result<(), ()> {
        let next_cur = self.cur.wrapping_sub(1);
        if next_cur < self.bot {
            return Err(());
        }
        self.cur = next_cur;
        unsafe {
            self.cur.write(word);
        }
        Ok(())
    }

    #[inline]
    pub fn pop(&mut self) -> Option<Word> {
        let next_cur = self.cur.wrapping_add(1);
        if next_cur > self.top {
            return None;
        }
        let val = unsafe {
            self.cur.read()
        };
        self.cur = next_cur;
        Some(val)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.cur = self.top;
    }
}

#[cfg(test)]
pub mod test {
    use std::mem::MaybeUninit;

    use crate::{Stack, Word};


    #[test]
    fn stack() {
        const ITEMS: usize = 16;
        let payload = Box::leak(Box::new(MaybeUninit::<[Word; ITEMS]>::uninit()))
            .as_mut_ptr()
            .cast();

        let mut stack = Stack::new(payload, ITEMS);

        for i in 0..(ITEMS as u32) {
            assert!(stack.push(Word::data(i)).is_ok());
        }
        assert!(stack.push(Word::data(100)).is_err());
        for i in (0..(ITEMS as u32)).rev() {
            assert_eq!(
                unsafe { stack.pop().unwrap().data },
                i
            );
        }
        assert!(stack.pop().is_none());
        unsafe {
            let _ = Box::<MaybeUninit::<[Word; ITEMS]>>::from_raw(payload.cast());
        }
    }
}
