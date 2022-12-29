pub struct Stack<T: Copy> {
    top: *mut T,
    cur: *mut T,
    bot: *mut T,
}

#[derive(Debug, PartialEq)]
pub enum StackError {
    StackEmpty,
    StackFull,
    OverwriteInvalid,
}

impl<T: Copy> Stack<T> {
    pub fn new(bottom: *mut T, items: usize) -> Self {
        let top = bottom.wrapping_add(items);
        debug_assert!(top >= bottom);
        Self {
            top,
            bot: bottom,
            cur: top,
        }
    }

    #[inline]
    pub fn push(&mut self, item: T) -> Result<(), StackError> {
        let next_cur = self.cur.wrapping_sub(1);
        if next_cur < self.bot {
            return Err(StackError::StackFull);
        }
        self.cur = next_cur;
        unsafe {
            self.cur.write(item);
        }
        Ok(())
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        let next_cur = self.cur.wrapping_add(1);
        if next_cur > self.top {
            return None;
        }
        let val = unsafe { self.cur.read() };
        self.cur = next_cur;
        Some(val)
    }

    #[inline]
    pub fn peek(&self) -> Option<T> {
        if self.cur == self.top {
            None
        } else {
            Some(unsafe { self.cur.read() })
        }
    }

    #[inline]
    pub fn peek_mut(&mut self) -> Option<&mut T> {
        if self.cur == self.top {
            None
        } else {
            Some(unsafe { &mut *self.cur })
        }
    }

    #[inline]
    pub fn peek_back_n(&self, n: usize) -> Option<T> {
        let request = self.cur.wrapping_add(n);
        if request >= self.top {
            None
        } else {
            unsafe { Some(request.read()) }
        }
    }

    #[inline]
    pub fn peek_back_n_mut(&mut self, n: usize) -> Option<&mut T> {
        let request = self.cur.wrapping_add(n);
        if request >= self.top {
            None
        } else {
            unsafe { Some(&mut *request) }
        }
    }

    #[inline]
    pub fn overwrite_back_n(&mut self, n: usize, item: T) -> Result<(), StackError> {
        let request = self.cur.wrapping_add(n);
        if request >= self.top {
            Err(StackError::OverwriteInvalid)
        } else {
            unsafe {
                request.write(item);
            }
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

        let mut stack = Stack::<Word>::new(payload.ptr(), payload.len());

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
