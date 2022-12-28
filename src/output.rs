pub struct OutputBuf {
    start: *mut u8,
    cur: *mut u8,
    end: *mut u8,
}

#[derive(Debug, PartialEq)]
pub enum OutputError {
    OutputFull,
    FormattingErr,
}

impl OutputBuf {
    pub fn new(bottom: *mut u8, size: usize) -> Self {
        let end = bottom.wrapping_add(size);
        debug_assert!(end >= bottom);
        Self {
            end,
            start: bottom,
            cur: bottom,
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        (self.end as usize) - (self.start as usize)
    }

    pub fn push_str(&mut self, stir: &str) -> Result<(), OutputError> {
        let bstr = stir.as_bytes();
        let new_end = self.cur.wrapping_add(bstr.len());
        if new_end > self.end {
            Err(OutputError::OutputFull)
        } else {
            unsafe {
                core::ptr::copy_nonoverlapping(bstr.as_ptr(), self.cur, bstr.len());
                self.cur = new_end;
            }
            Ok(())
        }
    }

    pub fn clear(&mut self) {
        self.cur = self.start;
    }

    pub fn as_str(&self) -> &str {
        let len = (self.cur as usize) - (self.start as usize);
        if len == 0 {
            ""
        } else {
            unsafe {
                let u8_sli = core::slice::from_raw_parts(self.start, len);
                core::str::from_utf8_unchecked(u8_sli)
            }
        }
    }
}

impl core::fmt::Write for OutputBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push_str(s).map_err(|_| core::fmt::Error)
    }
}
