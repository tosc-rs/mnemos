pub struct WordStrBuf {
    start: *mut u8,
    cur: *mut u8,
    end: *mut u8,
    holding: Holding,
}

enum Holding {
    None,
    Word((*mut u8, usize)),
    Str((*mut u8, usize)),
}

/// Errors returned by [`WordStrBuf::fill`].
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FillError {
    /// The [`WordStrBuf`] does not have sufficient capacity for the provided
    /// input.
    NoCapacity(usize),
    /// The input string contains non-ASCII characters.
    NotAscii,
}

/// Errors returned by [`WordStrBuf::advance_str`] indicating that an invalid
/// string literal was found.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StrLiteralError {
    /// The current word is not the beginning of a string literal (`."`).
    NotAStr,
    /// The `."` was not followed by anything.
    Empty,
    /// The string literal was not terminated by a `"`.
    Unterminated,
}

impl WordStrBuf {
    pub fn new(bottom: *mut u8, size: usize) -> Self {
        let end = bottom.wrapping_add(size);
        debug_assert!(end >= bottom);
        Self {
            end,
            start: bottom,
            cur: end,
            holding: Holding::None,
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        (self.end as usize) - (self.start as usize)
    }

    pub fn fill(&mut self, input: &str) -> Result<(), FillError> {
        let ilen = input.len();
        let cap = self.capacity();
        if ilen > cap {
            return Err(FillError::NoCapacity(cap));
        }
        if !input.is_ascii() {
            // TODO: Do I care about this?
            return Err(FillError::NotAscii);
        }
        // TODO: I probably *don't* want to lowercase everything, this also affects
        // things like string literals, which don't need to be lowercased.
        unsafe {
            let istart = input.as_bytes().as_ptr();
            for i in 0..ilen {
                self.start
                    .add(i)
                    .write((istart.add(i).read()).to_ascii_lowercase());
            }
            core::ptr::write_bytes(self.start.add(ilen), b' ', cap - ilen);
        }
        self.cur = self.start;
        Ok(())
    }

    // Move `self.cur` to the next non-whitespace character,
    // and return the value of `self.cur` after moving.
    //
    // Returns `None` if we hit the end.
    fn next_nonwhitespace(&mut self) -> Option<*mut u8> {
        loop {
            if self.cur == self.end {
                return None;
            }
            if !unsafe { *self.cur }.is_ascii_whitespace() {
                return Some(self.cur);
            }
            self.cur = self.cur.wrapping_add(1);
        }
    }

    pub fn advance(&mut self) {
        self.holding = Holding::None;

        // Find the start, skipping any ASCII whitespace
        let start = match self.next_nonwhitespace() {
            Some(s) => s,
            None => return,
        };
        // Find the end, either the first ASCII whitespace, or the end of the buffer
        // This is ONE PAST the last character
        let end = loop {
            if self.cur == self.end {
                break self.end;
            }
            if unsafe { *self.cur }.is_ascii_whitespace() {
                break self.cur;
            }
            self.cur = self.cur.wrapping_add(1);
        };
        let size = (end as usize) - (start as usize);
        self.holding = Holding::Word((start, size));
    }

    pub fn advance_str(&mut self) -> Result<(), StrLiteralError> {
        if self.cur_word() == Some(r#".""#) {
            self.holding = Holding::None;
        } else {
            return Err(StrLiteralError::NotAStr);
        }

        let start = match self.next_nonwhitespace() {
            Some(s) => s,
            None => return Err(StrLiteralError::Empty),
        };

        let end = loop {
            if self.cur == self.end {
                return Err(StrLiteralError::Unterminated);
            }
            if unsafe { *self.cur } == b'"' {
                // Move past the quote by one. Okay if this is now END.
                let pre_quote = self.cur;
                self.cur = self.cur.wrapping_add(1);
                break pre_quote;
            }
            self.cur = self.cur.wrapping_add(1);
        };

        let size = (end as usize) - (start as usize);
        self.holding = Holding::Str((start, size));
        Ok(())
    }

    pub fn cur_str_literal(&self) -> Option<&str> {
        match &self.holding {
            Holding::None => None,
            Holding::Str((start, len)) => Some(unsafe {
                let u8_sli = core::slice::from_raw_parts(*start, *len);
                core::str::from_utf8_unchecked(u8_sli)
            }),
            Holding::Word(_) => None,
        }
    }

    pub fn cur_word(&self) -> Option<&str> {
        match &self.holding {
            Holding::None => None,
            Holding::Word((start, len)) => Some(unsafe {
                let u8_sli = core::slice::from_raw_parts(*start, *len);
                core::str::from_utf8_unchecked(u8_sli)
            }),
            Holding::Str(_) => None,
        }
    }
}
