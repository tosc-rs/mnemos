use core::hash::Hasher as _;
use core::{marker::PhantomData, ops::Deref};
use hash32::{FnvHasher, Hasher};

pub struct TmpFaStr<'a> {
    stir: PhantomData<&'a str>,
    fastr: FaStr,
}

impl<'a> Deref for TmpFaStr<'a> {
    type Target = FaStr;

    fn deref(&self) -> &Self::Target {
        &self.fastr
    }
}

impl<'a> TmpFaStr<'a> {
    pub fn new_from(stir: &'a str) -> Self {
        let fastr = unsafe { FaStr::new(stir.as_ptr(), stir.len()) };
        Self {
            fastr,
            stir: PhantomData,
        }
    }
}

pub struct FaStr {
    ptr: *const u8,
    len_hash: LenHash,
}

impl FaStr {
    pub unsafe fn new(addr: *const u8, len: usize) -> Self {
        let u8_sli = core::slice::from_raw_parts(addr, len);
        let len_hash = LenHash::from_bstr(u8_sli);
        Self {
            ptr: addr,
            len_hash,
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_bytes(&self) -> &[u8] {
        let len = self.len_hash.len();
        unsafe { core::slice::from_raw_parts(self.ptr, len) }
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }

    pub fn raw(&self) -> u32 {
        self.len_hash.inner
    }

    /// Returns a copy of this `FaStr` pointing to the same data.
    ///
    /// # Safety
    ///
    /// This aliases the memory location pointed to by this `FaStr`, and
    /// therefore is subject to the same invariants as `FaStr::new` --- the
    /// memory area must live as long as the returned `FaStr` does. In practice
    /// this is safe to call when the caller knows that the `Dictionary` bump
    /// arena that `self` points into will live as long as the dictionary in
    /// which the returned `FaStr` will be stored --- such as when the `FaStr`'s
    /// dictionary is kept alive by a parent reference from a child dictionary
    /// in which the new `FaStr` will be used.
    pub(crate) unsafe fn copy_in_child(&self) -> Self {
        Self {
            ptr: self.ptr,
            len_hash: LenHash { inner: self.len_hash.inner },
        }
    }
}

impl PartialEq for FaStr {
    fn eq(&self, other: &Self) -> bool {
        // First, check the hash
        if self.len_hash.eq_ignore_bits(&other.len_hash) {
            // The hash matches, but there might be collisions. Do the strcmp
            // to make sure
            self.as_bytes().eq(other.as_bytes())
        } else {
            // If the hash doesn't match, it's definitely not equal.
            false
        }
    }
}

pub struct LenHash {
    // 29..32: 3-bit bitfield
    // 24..29: 5-bit len (0..31)
    // 00..24: 24-bit FnvHash
    inner: u32,
}

impl LenHash {
    const HASH_MASK: u32 = 0x00FF_FFFF;
    const BITS_MASK: u32 = 0xE000_0000;
    const LEN_MASK: u32 = 0x1F00_0000;

    /// Creates a new LenHash, considering UP TO 31 ascii characters.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self::from_bstr(s.as_bytes())
    }

    pub fn from_bstr(s: &[u8]) -> Self {
        let mut hasher = FnvHasher::default();
        let len = s.len().min(31);

        // TODO: I COULD hash more than 31 chars, which might give us some
        // chance of having longer strings, but we couldn't detect collisions
        // for strings longer than that. Maybe, but seems niche.
        hasher.write(&s[..len]);
        let hash = hasher.finish32();
        let inner = ((len as u32) << 24) | (hash & Self::HASH_MASK);
        Self { inner }
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        let len_u32 = (self.inner & Self::LEN_MASK) >> 24;
        len_u32 as usize
    }

    pub fn bits(&self) -> u8 {
        let bits_u32 = (self.inner & Self::BITS_MASK) >> 29;
        bits_u32 as u8
    }

    pub fn eq_ignore_bits(&self, other: &Self) -> bool {
        (self.inner & !Self::BITS_MASK) == (other.inner & !Self::BITS_MASK)
    }
}

pub const fn comptime_fastr(s: &'static str) -> FaStr {
    let len = s.len();
    assert!(!s.is_empty());
    assert!(len <= 31);
    let hash = comptime_hash_by(s.as_bytes(), BASIS);
    FaStr {
        ptr: s.as_ptr(),
        len_hash: LenHash {
            inner: ((len as u32) << 24) | (hash & LenHash::HASH_MASK),
        },
    }
}

const fn comptime_hash_by(sli: &'static [u8], state: u32) -> u32 {
    match sli.split_first() {
        Some((first, rest)) => {
            let state = state ^ (*first as u32);
            let state = state.wrapping_mul(PRIME);
            comptime_hash_by(rest, state)
        }
        None => state,
    }
}

const BASIS: u32 = 0x811c_9dc5;
const PRIME: u32 = 0x0100_0193;

#[cfg(test)]
pub mod test {
    use crate::fastr::FaStr;

    use super::{comptime_fastr, TmpFaStr};

    #[test]
    fn const_fastr() {
        use comptime_fastr as cf;
        const ITEMS: &[(&str, FaStr)] = &[
            ("hello", cf("hello")),
            ("this", cf("this")),
            ("is", cf("is")),
            ("a", cf("a")),
            ("very", cf("very")),
            ("silly", cf("silly")),
            ("test", cf("test")),
        ];

        for (txt, cf) in ITEMS {
            let tafs = TmpFaStr::new_from(txt);
            assert!(cf == &tafs.fastr);
        }
    }
}
