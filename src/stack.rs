use crate::word::Word;

pub struct Stack {
    top: *mut Word,
    cur: *mut Word,
    bot: *mut Word,
}

#[derive(Debug, PartialEq)]
pub enum StackError {
    StackEmpty,
    StackFull,
    OverwriteInvalid,
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
    pub fn push(&mut self, word: Word) -> Result<(), StackError> {
        let next_cur = self.cur.wrapping_sub(1);
        if next_cur < self.bot {
            return Err(StackError::StackFull);
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
        let val = unsafe { self.cur.read() };
        self.cur = next_cur;
        Some(val)
    }

    #[inline]
    pub fn peek(&self) -> Option<Word> {
        if self.cur == self.top {
            None
        } else {
            Some(unsafe { self.cur.read() })
        }
    }

    #[inline]
    pub fn peek_back_n(&self, n: usize) -> Option<Word> {
        let request = self.cur.wrapping_add(n);
        if request >= self.top {
            None
        } else {
            unsafe { Some(request.read()) }
        }
    }

    #[inline]
    pub fn overwrite_back_n(&mut self, n: usize, word: Word) -> Result<(), StackError> {
        let request = self.cur.wrapping_add(n);
        if request >= self.top {
            Err(StackError::OverwriteInvalid)
        } else {
            unsafe { request.write(word); }
            Ok(())
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.cur = self.top;
    }
}

#[cfg(test)]
pub mod test {
    use super::Stack;
    use crate::test::LeakBox;
    use crate::Word;

    #[test]
    fn stack() {
        const ITEMS: usize = 16;
        let payload: LeakBox<Word, ITEMS> = LeakBox::new();

        let mut stack = Stack::new(payload.ptr(), payload.len());

        for _ in 0..3 {
            for i in 0..(ITEMS as i32) {
                assert!(stack.push(Word::data(i)).is_ok());
            }
            assert!(stack.push(Word::data(100)).is_err());
            for i in (0..(ITEMS as i32)).rev() {
                assert_eq!(unsafe { stack.pop().unwrap().data }, i);
            }
            assert!(stack.pop().is_none());
        }
    }
}
