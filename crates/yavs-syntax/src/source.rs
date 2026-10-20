//! Source as the lexer and parser see it: the bytes on disk and how to read them.
//!
//! Offsets are always byte offsets into the file on disk. Characters are
//! decoded per encoding: a Latin-1 file has one character per byte, a UTF-8
//! file has multi-byte characters (checked to be in ISO-8859-1 where VHDL
//! gives them meaning, see the lexer).

use std::borrow::Cow;

#[derive(Clone, Copy)]
pub(crate) struct Src<'a> {
    pub bytes: &'a [u8],
    pub latin1: bool,
}

impl<'a> Src<'a> {
    /// The character starting at byte `pos`.
    pub fn char_at(&self, pos: usize) -> Option<char> {
        let b = *self.bytes.get(pos)?;
        if b < 0x80 || self.latin1 {
            return Some(b as char);
        }
        let len = match b {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return Some(char::REPLACEMENT_CHARACTER),
        };
        let s = self.bytes.get(pos..pos + len)?;
        std::str::from_utf8(s).ok()?.chars().next()
    }

    /// Bytes taken by `c` in this file.
    pub fn char_len(&self, c: char) -> usize {
        if self.latin1 { 1 } else { c.len_utf8() }
    }

    /// The text of a byte range.
    pub fn text(&self, start: u32, end: u32) -> Cow<'a, str> {
        let b = &self.bytes[start as usize..end as usize];
        if self.latin1 && !b.is_ascii() {
            Cow::Owned(b.iter().map(|&c| c as char).collect())
        } else {
            match std::str::from_utf8(b) {
                Ok(s) => Cow::Borrowed(s),
                Err(_) => String::from_utf8_lossy(b),
            }
        }
    }

    /// The byte range contains a line break.
    pub fn has_newline(&self, start: u32, end: u32) -> bool {
        self.bytes[start as usize..end as usize].contains(&b'\n')
    }
}
