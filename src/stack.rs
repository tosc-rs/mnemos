use crate::word::Word;

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
        let val = unsafe { self.cur.read() };
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
    use super::Stack;
    use crate::test::LeakBox;
    use crate::Word;

    #[test]
    fn stack() {
        const ITEMS: usize = 16;
        let payload: LeakBox<Word, ITEMS> = LeakBox::new();

        let mut stack = Stack::new(payload.ptr(), payload.len());

        for _ in 0..3 {
            for i in 0..(ITEMS as u32) {
                assert!(stack.push(Word::data(i)).is_ok());
            }
            assert!(stack.push(Word::data(100)).is_err());
            for i in (0..(ITEMS as u32)).rev() {
                assert_eq!(unsafe { stack.pop().unwrap().data }, i);
            }
            assert!(stack.pop().is_none());
        }
    }
}
