pub struct Name {
    prec_len: u8,
    name: [u8; 31],
}

impl Name {
    pub const fn new_from_arr(mode: Mode, len: usize, arr: [u8; 31]) -> Self {
        assert!(len <= 31);
        let prec_len = match mode {
            Mode::Run => len as u8,
            Mode::Compile => (len as u8) | 0x80,
        };
        let mut i = 0;
        while i < len {
            assert!(arr[i].is_ascii());
            i += 1;
        }
        Self {
            prec_len,
            name: arr,
        }
    }

    pub fn new_from_bstr(mode: Mode, bstr: &[u8]) -> Self {
        let len = bstr.len().min(31);
        let prec_len = match mode {
            Mode::Run => len as u8,
            Mode::Compile => (len as u8) | 0x80,
        };

        let mut new = Name {
            prec_len,
            name: [0u8; 31],
        };
        new.name[..len].copy_from_slice(&bstr[..len]);

        // TODO: Smarter way to make sure this is a str?
        debug_assert!({ (new.name[..len]).iter().all(|b| b.is_ascii()) });

        new
    }

    pub fn as_str(&self) -> &str {
        let len = (self.prec_len & 0x7F) as usize;
        unsafe { core::str::from_utf8_unchecked(&self.name[..len]) }
    }
}

// Is this just context?
pub enum Mode {
    Run,
    Compile,
}
