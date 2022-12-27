// (cond) IF (positive) THEN (end)
//   Needs: IF -> THEN distance for (negative) case
//
// (cond) IF (positive) ELSE (negative) THEN
//   Needs: IF -> THEN distance for (negative) case
//          ELSE -> THEN distance for (positive) case

use core::mem::transmute;

use crate::{word::Word, BuiltinFunc};

#[repr(C)]
#[derive(Copy, Clone)]
pub struct ShortWord {
    pub short: i16,
    pub _pad: u8,
    pub discrim: Discrim,
}

pub struct CFAIter {
    start: *mut Word,
    cur: *mut Word,
    end: *mut Word,
}

impl CFAIter {
    pub fn from_cfa(start: *mut Word) -> Result<Self, ()> {
        // CFA arrays always start with a length word. Len is how many WORDS
        // there are, not how many ELEMENTS. So if there is a 2-word element
        // and a 1-word element, the length is 3, not 2.
        let len = unsafe { (*start).data };
        if len.is_negative() {
            return Err(());
        }
        let cur = unsafe { start.add(1) };
        let end = unsafe { cur.add(len as usize) };

        Ok(Self {
            start: cur,
            cur,
            end,
        })
    }

    fn pop(&mut self) -> Option<Word> {
        if self.cur == self.end {
            return None;
        }
        unsafe {
            let ret = *self.cur;
            self.cur = self.cur.add(1);
            Some(ret)
        }
    }

    fn short_to_len(&mut self, len: i16) -> Option<usize> {
        match usize::try_from(len) {
            Ok(len) => Some(len),
            Err(_) => {
                debug_assert!(false, "Invalid CFA len? {:04X}", len);
                self.fuse_err();
                None
            }
        }
    }

    pub fn clone_from_start(&self) -> Self {
        Self {
            start: self.start,
            cur: self.start,
            end: self.end,
        }
    }

    /// We have had a bad time, and we will be giving no more values.
    #[inline]
    fn fuse_err(&mut self) {
        self.cur = self.end;
    }

    pub fn next(&mut self) -> Option<CfaWord> {
        let hdr = unsafe { self.pop()?.hdr };
        match hdr.discrim {
            Discrim::ShortLiteral => Some(CfaWord::Literal(hdr.short.into())),
            Discrim::LongLiteral => {
                let lit = unsafe { self.pop()?.data };
                Some(CfaWord::Literal(lit))
            }
            Discrim::LongInterpret => {
                let ptr: *mut () = unsafe { self.pop()?.ptr };
                let cfa: *mut Word = ptr.cast();
                let len = self.short_to_len(hdr.short)?;
                Some(CfaWord::Interpret { len, cfa })
            }
            Discrim::LongBuiltin => {
                let ptr: *mut () = unsafe { self.pop()?.ptr };
                let bin: BuiltinFunc<'static, 'static> = unsafe { transmute(ptr) };
                Some(CfaWord::Builtin { bin })
            },
            #[allow(unreachable_patterns)]
            _ => {
                debug_assert!(false, "Invalid CFA word? {:02X}", hdr.discrim as u8);
                self.fuse_err();
                return None;
            }
        }
    }
}

pub enum CfaWord {
    Literal(i32),
    Interpret { len: usize, cfa: *mut Word },
    Builtin { bin: BuiltinFunc<'static, 'static> },
}

pub enum Encoded {
    EncodeError,
    OneWord(Word),
    TwoWord((Word, Word)),
}

impl CfaWord {
    pub fn encode(&self) -> Encoded {
        match self {
            CfaWord::Literal(val) => match i16::try_from(*val) {
                Ok(short) => {
                    let hdr = ShortWord { short, _pad: 0, discrim: Discrim::ShortLiteral };
                    Encoded::OneWord(Word::hdr(hdr))
                },
                Err(_) => {
                    let hdr = ShortWord { short: 0, _pad: 0, discrim: Discrim::LongLiteral };
                    let w1 = Word::hdr(hdr);
                    let w2 = Word::data(*val);
                    Encoded::TwoWord((w1, w2))
                },
            },
            CfaWord::Interpret { len, cfa } => {
                match i16::try_from(*len) {
                    Ok(slen) => {
                        let hdr = ShortWord { short: slen, _pad: 0, discrim: Discrim::LongInterpret };
                        let w1 = Word::hdr(hdr);
                        let w2 = Word::ptr(*cfa);
                        Encoded::TwoWord((w1, w2))
                    },
                    Err(_) => Encoded::EncodeError,
                }
            },
            CfaWord::Builtin { bin } => {
                let hdr = ShortWord { short: 0, _pad: 0, discrim: Discrim::LongBuiltin };
                let w1 = Word::hdr(hdr);
                let w2 = Word::ptr(*bin as *mut ());
                Encoded::TwoWord((w1, w2))
            },
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, PartialEq)]
pub enum Discrim {
    // 1-word Words
    ShortLiteral = 0,

    // 2-word Words
    LongLiteral = 128,
    LongInterpret,
    LongBuiltin,
}

#[cfg(test)]
pub mod test {
    use crate::cfa::{Discrim, ShortWord};
    use core::mem::size_of;

    #[test]
    fn size() {
        assert_eq!(size_of::<Discrim>(), size_of::<u8>());
        assert_eq!(size_of::<ShortWord>(), size_of::<u32>());
    }
}
