pub struct WordStrBuf {
    start: *mut u8,
    cur: *mut u8,
    end: *mut u8,
    cur_word: Option<(*mut u8, usize)>,
}

impl WordStrBuf {
    pub fn new(bottom: *mut u8, size: usize) -> Self {
        let end = bottom.wrapping_add(size);
        debug_assert!(end >= bottom);
        Self {
            end,
            start: bottom,
            cur: end,
            cur_word: None,
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        (self.end as usize) - (self.start as usize)
    }

    pub fn fill(&mut self, input: &str) -> Result<(), ()> {
        let ilen = input.len();
        let cap = self.capacity();
        if ilen > cap {
            return Err(());
        }
        if !input.is_ascii() {
            // TODO: Do I care about this?
            return Err(());
        }
        unsafe {
            let istart = input.as_bytes().as_ptr();
            for i in 0..ilen {
                self.start
                    .add(i)
                    .write((*istart.add(i)).to_ascii_lowercase());
            }
            core::ptr::write_bytes(self.start.add(ilen), b' ', cap - ilen);
        }
        self.cur = self.start;
        Ok(())
    }

    pub fn advance(&mut self) {
        let _ = self.cur_word.take();

        // Find the start, skipping any ASCII whitespace
        let start = loop {
            if self.cur == self.end {
                return;
            }
            if !unsafe { *self.cur }.is_ascii_whitespace() {
                break self.cur;
            }
            self.cur = self.cur.wrapping_add(1);
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
        self.cur_word = Some((start, size));
    }

    pub fn cur_word(&self) -> Option<&str> {
        self.cur_word.map(|(start, len)| unsafe {
            let u8_sli = core::slice::from_raw_parts(start, len);
            core::str::from_utf8_unchecked(u8_sli)
        })
    }
}
