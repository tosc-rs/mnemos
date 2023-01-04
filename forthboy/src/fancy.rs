use std::cmp::Ordering;

use crate::bricks::Bricks;

#[derive(Debug, PartialEq)]
pub enum LineError {
    Full,
    InvalidChar,
    ReadOnly,
    WriteGap,
}

#[derive(Debug, PartialEq, Copy, Clone)]
#[repr(u8)]
pub enum Source {
    Local,
    Remote,
}

#[derive(Debug)]
pub struct Line<const C: usize> {
    fill: u8,
    buf: [u8; C],
    pub status: Source,
}

impl<const C: usize> Line<C> {
    pub const fn new() -> Self {
        Self {
            fill: 0,
            buf: [0u8; C],
            status: Source::Local,
        }
    }

    pub fn clear(&mut self) {
        self.fill = 0;
        self.status = Source::Local;
    }

    pub fn len(&self) -> usize {
        self.fill.into()
    }

    pub fn is_empty(&self) -> bool {
        self.fill == 0
    }

    pub fn is_full(&self) -> bool {
        self.len() >= C
    }

    pub fn pop(&mut self) {
        if self.fill != 0 {
            self.fill -= 1;
        }
    }

    pub fn as_str(&self) -> &str {
        self.buf
            .get(..self.len())
            .and_then(|s| core::str::from_utf8(s).ok())
            .unwrap_or("")
    }

    pub const fn cap_u8() -> u8 {
        if C > ((u8::MAX - 1) as usize) {
            panic!("Too big!")
        } else {
            C as u8
        }
    }

    pub fn extend(&mut self, s: &str) -> Result<(), LineError> {
        let len = self.len();

        if len + s.len() > C {
            return Err(LineError::Full);
        }
        if !s.as_bytes().iter().copied().all(acceptable_ascii) {
            return Err(LineError::InvalidChar);
        }
        self.buf[len..][..s.len()].copy_from_slice(s.as_bytes());
        self.fill += s.len() as u8;
        Ok(())
    }

    pub fn overwrite(&mut self, pos: usize, ovrw: u8) -> Result<(), LineError> {
        if pos > self.len() || pos >= C {
            return Err(LineError::Full);
        }
        ascii_good(ovrw)?;

        self.buf[pos] = ovrw;
        if pos == self.len() {
            self.fill += 1;
        }
        Ok(())
    }

    pub fn not_full(&self) -> Result<(), LineError> {
        if self.is_full() {
            Err(LineError::Full)
        } else {
            Ok(())
        }
    }

    pub fn push(&mut self, ins: u8) -> Result<(), LineError> {
        self.not_full()?;
        ascii_good(ins)?;
        self.buf[self.len()] = ins;
        self.fill += 1;
        Ok(())
    }

    pub fn insert(&mut self, pos: usize, ins: u8) -> Result<(), LineError> {
        self.not_full()?;

        if pos >= C {
            return Err(LineError::Full);
        }
        if !acceptable_ascii(ins) {
            return Err(LineError::InvalidChar);
        }

        match self.len().cmp(&pos) {
            Ordering::Equal => {
                self.buf[pos] = ins;
                self.fill += 1;
                Ok(())
            }
            Ordering::Greater => {
                let len = self.len();
                self.buf[len] = ins;
                rot_right(&mut self.buf[..len + 1]);
                self.fill += 1;
                Ok(())
            }
            Ordering::Less => return Err(LineError::WriteGap), // trying to insert AFTER the "tip"
        }
    }
}

fn ascii_good(c: u8) -> Result<(), LineError> {
    if acceptable_ascii(c) {
        Ok(())
    } else {
        Err(LineError::InvalidChar)
    }
}

fn acceptable_ascii(c: u8) -> bool {
    c.is_ascii() && !c.is_ascii_control()
}

#[inline]
pub(crate) fn rot_right<T: Sized>(sli: &mut [T]) {
    let len = sli.len();
    if len <= 1 {
        // Look, it's rotated!
        return;
    }
    unsafe {
        let ptr = sli.as_mut_ptr();
        let last_val = ptr.add(len - 1).read();
        core::ptr::copy(ptr, ptr.add(1), len - 1);
        ptr.write(last_val);
    }
}

#[inline]
pub(crate) fn rot_left<T: Sized>(sli: &mut [T]) {
    let len = sli.len();
    if len <= 1 {
        // Look, it's rotated!
        return;
    }
    unsafe {
        let ptr = sli.as_mut_ptr();
        let first_val = ptr.read();
        core::ptr::copy(ptr.add(1), ptr, len - 1);
        ptr.add(len - 1).write(first_val);
    }
}

/*

Reference coordinates:

oldest
^
|
|
|
|
0---------->rightmost

*/

// pub struct RLinesOldToNew<'a, const L: usize, const C: usize> {
//     rl: &'a RingLine<L, C>,
//     len_idx: u8,
// }

// impl<'a, const L: usize, const C: usize> Iterator for RLinesOldToNew<'a, L, C> {
//     type Item = (Status, &'a str);

//     fn next(&mut self) -> Option<Self::Item> {
//         if self.len_idx >= self.rl.len {
//             None
//         } else {
//             let wlen = (usize::from(self.rl.tail) + usize::from(self.len_idx)) % L;
//             self.len_idx += 1;
//             let line = &self.rl.lines[wlen];
//             Some((line.status, line.as_str()))
//         }
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) {
//         let remain = self.rl.len.checked_sub(self.len_idx).unwrap_or(0) as usize;
//         (remain, Some(remain))
//     }
// }

// pub struct RLinesNewToOld<'a, const L: usize, const C: usize> {
//     rl: NonNull<RingLine<L, C>>,
//     _pd: PhantomData<&'a mut RingLine<L, C>>,
//     len_idx: u8,
//     done: bool,
// }

// impl<'a, const L: usize, const C: usize> Iterator for RLinesNewToOld<'a, L, C> {
//     type Item = &'a mut Line<C>;

//     fn next(&mut self) -> Option<Self::Item> {
//         if self.done {
//             None
//         } else {
//             unsafe {
//                 let tail = self.rl.as_ref().tail;
//                 let wlen = (usize::from(tail) + usize::from(self.len_idx)) % L;
//                 if self.len_idx == 0 {
//                     self.done = true;
//                 } else {
//                     self.len_idx -= 1;
//                 }
//                 Some(&mut *addr_of_mut!((*self.rl.as_ptr()).lines).cast::<Line<C>>().add(wlen))
//             }

//         }
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) {
//         let remain = usize::from(self.len_idx);
//         (remain, Some(remain))
//     }
// }

#[derive(Debug)]
pub struct RingLine<const L: usize, const C: usize> {
    pub lines: [Line<C>; L],
    pub brick: Bricks<L>,
}

#[derive(Debug, PartialEq)]
pub enum RingLineError {
    Line(LineError),
}

impl From<LineError> for RingLineError {
    fn from(le: LineError) -> Self {
        RingLineError::Line(le)
    }
}

impl<const L: usize, const C: usize> RingLine<L, C> {
    const ONELINE: Line<C> = Line::<C>::new();
    const INIT: [Line<C>; L] = [Self::ONELINE; L];

    pub fn new() -> Self {
        Self {
            lines: Self::INIT,
            brick: Bricks::new(),
        }
    }

    // pub fn is_empty(&self) -> bool {
    //     self.len == 0
    // }

    // pub fn is_full_lines(&self) -> bool {
    //     usize::from(self.len) == L
    // }

    // fn cur_head_idx(&self) -> Option<usize> {
    //     if self.is_empty() {
    //         None
    //     } else {
    //         let wlen = usize::from(self.tail) + usize::from(self.len - 1);
    //         Some(wlen % L)
    //     }
    // }

    // fn advance(&mut self) -> usize {
    //     let wlen = (usize::from(self.tail) + usize::from(self.len)) % L;
    //     if self.is_full_lines() {
    //         println!("Advancing Tail, wlen: {}", wlen);
    //         let lu8 = L as u8;
    //         self.tail = (self.tail + 1) % lu8;
    //         self.lines[wlen].clear();
    //     } else {
    //         println!("Advancing Len, wlen: {}", wlen);
    //         self.len += 1;
    //     }
    //     wlen
    // }

    fn get_first_writeable(&mut self) -> Option<&mut Line<C>> {
        let Self { lines, brick } = self;
        // If empty, make a new one and return
        // If not empty, is the head writable and !full? => return
        // else, if not full make a new one and return
        // else, remove oldest, make a new one and return
        let mut new = false;
        let wr = if let Some(wr) = brick.ue_front() {
            let cur = &lines[wr];
            if cur.is_full() {
                new = true;
                self.brick.insert_ue_front().ok()?
            } else {
                wr
            }
        } else {
            new = true;
            self.brick.insert_ue_front().ok()?
        };
        let cur = &mut lines[wr];
        if new {
            cur.clear();
            cur.status = Source::Local;
        }

        Some(cur)
    }

    pub fn append_char(&mut self, c: u8) -> Result<(), RingLineError> {
        self.get_first_writeable().unwrap().push(c)?;
        Ok(())
    }

    pub fn pop_char(&mut self) {
        let Self { lines, brick } = self;
        if let Some(cur) = brick.iter_user_editable_mut(lines).next() {
            if cur.is_empty() {
                brick.pop_ue_front();
            } else {
                cur.pop();
            }
        }
    }

    // pub fn iter_old_to_new(&self) -> RLinesOldToNew<'_, L, C> {
    //     RLinesOldToNew {
    //         rl: self,
    //         len_idx: 0,
    //     }
    // }

    // pub fn iter_new_to_old(&mut self) -> RLinesNewToOld<'_, L, C> {
    //     RLinesNewToOld {
    //         done: self.is_empty(),
    //         len_idx: self.len.checked_sub(1).unwrap_or(0),
    //         rl: NonNull::from(self),
    //         _pd: PhantomData,
    //     }
    // }
}

#[cfg(test)]
mod test {
    use crate::fancy::{LineError, Source};

    use super::{Line, RingLine};

    #[test]
    fn ascii() {
        assert!(b'\r'.is_ascii());
        assert!(b'\r'.is_ascii_control());
        assert!(b'\n'.is_ascii());
        assert!(b'\n'.is_ascii_control());
    }

    #[test]
    fn smoke_ring() {}

    #[test]
    fn smoke_line() {
        let mut line = Line::<10>::new();
        assert_eq!(line.as_str(), "");
        for (i, c) in b"hello".iter().enumerate() {
            line.insert(i, *c).unwrap();
            assert_eq!(line.as_str(), &"hello"[..(i + 1)]);
        }
        for i in (line.len() + 1)..256 {
            assert!(matches!(
                line.insert(i, b' ').unwrap_err(),
                LineError::WriteGap | LineError::Full
            ));
        }
        for c in b"world" {
            line.insert(0, *c).unwrap();
        }
        assert_eq!(line.as_str(), "dlrowhello");
        for i in 0..256 {
            assert_eq!(line.insert(i, b' ').unwrap_err(), LineError::Full);
        }
        for i in 0..line.len() {
            line.overwrite(i, b'a').unwrap();
        }
        assert_eq!(line.as_str(), "aaaaaaaaaa");
        for i in line.len()..256 {
            assert_eq!(line.insert(i, b' ').unwrap_err(), LineError::Full);
        }

        line.clear();
        assert_eq!(line.as_str(), "");
        for i in 1..256 {
            assert!(matches!(
                line.overwrite(i, b' ').unwrap_err(),
                LineError::WriteGap | LineError::Full
            ));
            assert!(matches!(
                line.insert(i, b' ').unwrap_err(),
                LineError::WriteGap | LineError::Full
            ));
        }
        line.overwrite(0, b'a').unwrap();
        assert_eq!(line.as_str(), "a");
        line.overwrite(0, b'b').unwrap();
        assert_eq!(line.as_str(), "b");
        line.clear();
        line.extend("hello").unwrap();
        line.extend("world").unwrap();
        assert_eq!(line.as_str(), "helloworld");

        line.pop();
        assert_eq!(line.as_str(), "helloworl");

        line.pop();
        assert_eq!(line.as_str(), "hellowor");

        line.clear();
        assert_eq!(
            line.extend("hello\nworl").unwrap_err(),
            LineError::InvalidChar
        );
        assert_eq!(
            line.extend("hello\rworl").unwrap_err(),
            LineError::InvalidChar
        );
        assert_eq!(line.extend("Späti").unwrap_err(), LineError::InvalidChar);
        assert_eq!(line.as_str(), "");
    }
}
