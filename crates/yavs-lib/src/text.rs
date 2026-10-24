//! The text of the index and of the records: a subset of TOML that is written and read by line,
//! with the strings written by [`put_str`] (a quote, a backslash or a control character is escaped).

/// `s` in double quotes.
pub(crate) fn put_str(o: &mut String, s: &str) {
    o.push('"');
    if s.bytes()
        .all(|b| b >= b' ' && b != 0x7f && b != b'"' && b != b'\\')
    {
        o.push_str(s);
    } else {
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                c if c < ' ' || c == '\x7f' => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
    }
    o.push('"');
}

/// `key = "s"` and a newline.
pub(crate) fn put_line(o: &mut String, key: &str, s: &str) {
    o.push_str(key);
    o.push_str(" = ");
    put_str(o, s);
    o.push('\n');
}

/// `key = n` and a newline.
pub(crate) fn put_num(o: &mut String, key: &str, n: impl std::fmt::Display) {
    use std::fmt::Write;
    let _ = writeln!(o, "{key} = {n}");
}

/// A position in the value of a line.
pub(crate) struct Cur<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cur<'a> {
    pub(crate) fn new(s: &'a str) -> Self {
        Cur { s, i: 0 }
    }

    pub(crate) fn eat(&mut self, c: u8) -> Option<()> {
        (self.s.as_bytes().get(self.i) == Some(&c)).then(|| self.i += 1)
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.i).copied()
    }

    pub(crate) fn done(&self) -> bool {
        self.i == self.s.len()
    }

    /// The offset of the next byte to read.
    pub(crate) fn pos(&self) -> usize {
        self.i
    }

    /// A double-quoted string with nothing to unescape, as it is in the text (`None`: not a
    /// string, or one with an escape, which `string` reads).
    pub(crate) fn plain(&mut self) -> Option<&'a str> {
        let b = self.s.as_bytes();
        (b.get(self.i) == Some(&b'"')).then_some(())?;
        let from = self.i + 1;
        let n = plain_len(&b[from..]);
        (b.get(from + n) == Some(&b'"')).then_some(())?;
        self.i = from + n + 1;
        Some(&self.s[from..from + n])
    }

    /// A double-quoted string, appended to `out` without its quotes and escapes.
    pub(crate) fn string(&mut self, out: &mut String) -> Option<()> {
        self.eat(b'"')?;
        let (s, b) = (self.s, self.s.as_bytes());
        let mut run = self.i;
        let mut j = self.i;
        loop {
            // The plain bytes of the string in one go.
            j += plain_len(&b[j..]);
            match *b.get(j)? {
                b'"' => {
                    out.push_str(&s[run..j]);
                    self.i = j + 1;
                    return Some(());
                }
                b'\\' => {
                    out.push_str(&s[run..j]);
                    match *b.get(j + 1)? {
                        c @ (b'"' | b'\\') => out.push(c as char),
                        b'u' => out.push(char::from_u32(
                            u32::from_str_radix(s.get(j + 2..j + 6)?, 16).ok()?,
                        )?),
                        _ => return None,
                    }
                    j += if b[j + 1] == b'u' { 6 } else { 2 };
                    run = j;
                }
                _ => return None,
            }
        }
    }
}

/// How many bytes of `b` come before the first quote, backslash or control character (all of
/// them when there is none): eight at a time while none of the eight is one.
fn plain_len(b: &[u8]) -> usize {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    // Whether some byte of `x` is zero.
    let zero = |x: u64| x.wrapping_sub(LO) & !x & HI != 0;
    let mut i = 0;
    while let Some(w) = b.get(i..i + 8) {
        let w = u64::from_le_bytes(w.try_into().expect("eight bytes"));
        // A byte below a space is a control character (a byte of 0x80 or more is never "below").
        let ctl = w.wrapping_sub(LO * 0x20) & !w & HI != 0;
        if ctl || zero(w ^ (LO * 0x22)) || zero(w ^ (LO * 0x5c)) {
            break;
        }
        i += 8;
    }
    i + b[i..]
        .iter()
        .take_while(|&&c| c != b'"' && c != b'\\' && c >= b' ')
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_round_trip() {
        for s in [
            "",
            "plain",
            "a\"b",
            "a\\b",
            "\u{e9}t\u{e9}",
            "tab\t",
            "nl\n",
            "\u{7f}",
            "\\u0041",
            "mix \"\\\n\u{1}\u{e9}",
        ] {
            let mut o = String::new();
            put_str(&mut o, s);
            assert!(o.bytes().all(|b| b >= b' ' && b != 0x7f), "{o:?}");
            let mut c = Cur::new(&o);
            let mut back = String::new();
            assert_eq!(c.string(&mut back), Some(()));
            assert!(c.done());
            assert_eq!(back, s);
        }
    }

    #[test]
    fn plain_strings_and_where_they_end() {
        assert_eq!(Cur::new("\"abc\" x").plain(), Some("abc"));
        assert_eq!(Cur::new("\"\"").plain(), Some(""));
        for s in ["x", "\"open", "\"a\\\"b\"", "\"a\tb\"", "\"\\u0041\""] {
            assert_eq!(Cur::new(s).plain(), None, "{s:?}");
        }
        // A special byte anywhere among plain ones, also past a multi-byte character.
        for len in 0..40 {
            for at in 0..len {
                for special in [b'"', b'\\', 0x1f, 0] {
                    let mut b = vec![b'a'; len];
                    b[at] = special;
                    assert_eq!(plain_len(&b), at, "{len} {at} {special}");
                    let mut b = "\u{e9}".repeat(len).into_bytes();
                    b.insert(at, special);
                    assert_eq!(plain_len(&b), at, "{len} {at} {special} (e-acute)");
                }
            }
            assert_eq!(plain_len(&vec![b'z'; len]), len);
        }
    }

    #[test]
    fn bad_strings() {
        for s in [
            "",
            "x",
            "\"open",
            "\"a\\\"",
            "\"a\\q\"",
            "\"\\u12\"",
            "\"\\ud800\"",
            "\"a\tb\"",
        ] {
            let mut out = String::new();
            assert_eq!(Cur::new(s).string(&mut out), None, "{s:?}");
        }
    }
}
