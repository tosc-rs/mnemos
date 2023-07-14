//! Keyboard events
//!
//! This module contains types representing keyboard events that are published
//! over a [`KeySubscription`](super::KeySubscription).
//!
//! The structure of the keyboard event API is based loosely on the interface
//! provided by the [`crossterm`] crate (MIT-licensed), with some modifications.
//!
//! [`crossterm`]: https://github.com/crossterm-rs/crossterm/blob/1efdce7ef63cba6992729db5f22262a60936fa8b/src/event.rs

/// A keyboard event.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct KeyEvent {
    /// What keyboard event occurred?
    pub kind: Kind,
    /// What modifier keys (if any) were held when the key event occurred?
    pub modifiers: Modifiers,
    /// What key code was pressed?
    pub code: KeyCode,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Pressed,
    Released,
    Held,
}

mycelium_bitfield::bitfield! {
    // Copy, Clone, and Debug are derived for us by `mycelium_bitfield`.
    #[derive(PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct Modifiers<u8> {
        pub const SHIFT: bool;
        pub const CTRL: bool;
        pub const ALT: bool;
        pub const META: bool;
        pub const CAPSLOCK: bool;
        pub const NUMLOCK: bool;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum KeyCode {
    /// Backspace key.
    Backspace,
    /// Enter key.
    Enter,
    /// Left arrow key.
    Left,
    /// Right arrow key.
    Right,
    /// Up arrow key.
    Up,
    /// Down arrow key.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page up key.
    PageUp,
    /// Page down key.
    PageDown,
    /// Tab key.
    Tab,
    /// Shift + Tab key.
    BackTab,
    /// Delete key.
    Delete,
    /// Insert key.
    Insert,
    /// F key.
    ///
    /// `KeyCode::F(1)` represents F1 key, etc.
    F(u8),
    /// A character.
    ///
    /// `KeyCode::Char('c')` represents `c` character, etc.
    Char(char),
    /// Null.
    Null,
    /// Escape key.
    Esc,
    /// Num Lock key.
    NumLock,
    /// Print Screen key.
    PrintScreen,
    /// Pause key.
    Pause,
    /// Menu key.
    Menu,
    /// The "Begin" key (often mapped to the 5 key when Num Lock is turned on).
    KeypadBegin,
    /// A media key.
    Media(MediaKeyCode),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MediaKeyCode {
    /// Play media key.
    Play,
    /// Pause media key.
    Pause,
    /// Play/Pause media key.
    PlayPause,
    /// Reverse media key.
    Reverse,
    /// Stop media key.
    Stop,
    /// Fast-forward media key.
    FastForward,
    /// Rewind media key.
    Rewind,
    /// Next-track media key.
    TrackNext,
    /// Previous-track media key.
    TrackPrevious,
    /// Record media key.
    Record,
    /// Lower-volume media key.
    LowerVolume,
    /// Raise-volume media key.
    RaiseVolume,
    /// Mute media key.
    MuteVolume,
}

impl KeyCode {
    #[must_use]
    pub fn into_char(self) -> Option<char> {
        match self {
            KeyCode::Char(c) => Some(c),
            KeyCode::Enter => Some('\n'),
            KeyCode::Tab => Some('\t'),
            KeyCode::Null => Some('\0'),
            KeyCode::Backspace => Some('\x7f'),
            _ => None,
        }
    }
}

impl KeyEvent {
    #[must_use]
    #[inline]
    pub fn into_char(self) -> Option<char> {
        self.code.into_char()
    }

    #[must_use]
    pub fn from_char(c: char) -> Self {
        Self {
            kind: Kind::Pressed,
            modifiers: Modifiers::new(),
            code: KeyCode::Char(c),
        }
    }

    #[must_use]
    pub fn from_ascii(c: u8, kind: Kind) -> Option<Self> {
        if !c.is_ascii() {
            return None;
        }
        Some(Self {
            kind,
            modifiers: Modifiers::new(),
            code: KeyCode::Char(c as char),
        })
    }
}

impl From<char> for KeyEvent {
    fn from(c: char) -> Self {
        Self::from_char(c)
    }
}
