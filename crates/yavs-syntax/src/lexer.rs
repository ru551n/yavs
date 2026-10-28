//! Hand-written lexer. Produces every token of a file, ending with `Eof`.

use yavs_diag::{Diagnostic, FileId, Span};

use crate::Std;
use crate::codes;
use crate::protect;
use crate::source::Src;
use crate::token::{Kw, Tok, Token};

pub(crate) fn lex(
    file: FileId,
    src: Src,
    std: Std,
    psl: bool,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Token> {
    let mut lx = Lexer {
        file,
        src,
        b: src.bytes,
        pos: 0,
        std,
        psl,
        diags,
        toks: Vec::with_capacity(src.bytes.len() / 4),
        pragma: Pragma::Off,
    };
    lx.run();
    lx.toks
}

struct Lexer<'a> {
    file: FileId,
    src: Src<'a>,
    b: &'a [u8],
    pos: usize,
    std: Std,
    /// `--psl`: `-- psl` comments are pragmas.
    psl: bool,
    diags: &'a mut Vec<Diagnostic>,
    toks: Vec<Token>,
    pragma: Pragma,
}

/// Where the lexer is relative to a `-- psl` comment pragma: its text is lexed as tokens, and
/// a statement it leaves open (no `;` yet) continues in the following comments.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pragma {
    Off,
    /// Inside the text of a pragma comment line.
    Line,
    /// After a pragma line whose statement has no `;` yet.
    Open,
}

/// Bit string base specifiers, lower case.
const BASES: [&str; 10] = ["b", "o", "x", "d", "ub", "uo", "ux", "sb", "so", "sx"];

fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

impl Lexer<'_> {
    fn peek(&self, off: usize) -> u8 {
        self.b.get(self.pos + off).copied().unwrap_or(0)
    }

    /// A bit string base specifier of the file's standard: 1993 has `b`, `o` and `x` only.
    fn is_base(&self, lower: &[u8]) -> bool {
        let n = if self.std >= Std::Vhdl2008 {
            BASES.len()
        } else {
            3
        };
        BASES.iter().take(n).any(|b| b.as_bytes() == lower)
    }

    fn char_at(&self, pos: usize) -> Option<char> {
        self.src.char_at(pos)
    }

    /// Report a character VHDL does not allow outside comments, and skip it.
    fn unexpected(&mut self, start: usize) {
        let ch = self.char_at(start).unwrap_or(char::REPLACEMENT_CHARACTER);
        self.pos = start + self.src.char_len(ch).max(1);
        self.error(
            start,
            self.pos,
            &format!("unexpected character `{ch}` (U+{:04X})", ch as u32),
            "not valid in VHDL",
        );
    }

    /// In a UTF-8 file, every character of a literal or identifier must be
    /// in ISO-8859-1, since VHDL values are Latin-1 characters.
    fn check_latin1(&mut self, start: usize, end: usize, what: &str) {
        if self.src.latin1 || self.b[start..end].is_ascii() {
            return;
        }
        let mut p = start;
        while p < end {
            let ch = self.char_at(p).unwrap_or(char::REPLACEMENT_CHARACTER);
            let len = self.src.char_len(ch).max(1);
            if ch as u32 > 0xff {
                self.diags.push(
                    Diagnostic::error(
                        codes::NOT_LATIN1,
                        format!("character `{ch}` (U+{:04X}) is not in ISO-8859-1", ch as u32),
                    )
                    .with_primary(Span::new(self.file, p as u32, (p + len) as u32), format!("in this {what}"))
                    .with_help(
                        "VHDL characters are ISO-8859-1 and have no escapes: use only Latin-1 characters \
                         here, or pass `--encoding=latin1` to read each byte as one character",
                    ),
                );
                return;
            }
            p += len;
        }
    }

    fn error(&mut self, start: usize, end: usize, msg: &str, label: &str) {
        self.diags.push(
            Diagnostic::error(codes::LEX, msg)
                .with_primary(Span::new(self.file, start as u32, end as u32), label),
        );
    }

    /// LRM 15.4.1, 15.5, 15.8 (1993 13.3.1, 13.4, 13.7): an underline stands between two
    /// letters or digits of an identifier or abstract literal, and between two characters of a
    /// bit value (#807). `inner` is the bit value of a bit string literal.
    fn check_underlines(&mut self, start: usize, end: usize, inner: bool, what: &str) {
        let s = &self.b[start..end];
        // The PSL keywords `until_` and `before_` (PSL 1.1 in 2008, 1.2).
        if !inner && (s.eq_ignore_ascii_case(b"until_") || s.eq_ignore_ascii_case(b"before_")) {
            return;
        }
        let near = |c: u8| {
            if inner {
                c != b'_'
            } else {
                c.is_ascii_alphanumeric() || c >= 0x80
            }
        };
        let bad = s.iter().enumerate().position(|(i, &c)| {
            c == b'_' && !(i > 0 && i + 1 < s.len() && near(s[i - 1]) && near(s[i + 1]))
        });
        if let Some(i) = bad {
            let between = if inner {
                "two of its digits"
            } else {
                "two letters or digits"
            };
            let msg = format!("an underline in {what} must stand between {between}");
            self.error(start + i, start + i + 1, &msg, "misplaced underline");
        }
    }

    fn push(&mut self, kind: Tok, start: usize) {
        if self.pragma == Pragma::Open {
            self.pragma = Pragma::Off;
        }
        self.toks.push(Token {
            kind,
            start: start as u32,
            end: self.pos as u32,
        });
    }

    fn run(&mut self) {
        while self.pos < self.b.len() {
            let start = self.pos;
            let c = self.b[self.pos];
            match c {
                b'\n' if self.pragma == Pragma::Line => {
                    self.pos += 1;
                    let open = self.toks.last().is_some_and(|t| t.kind != Tok::Semi);
                    self.pragma = if open { Pragma::Open } else { Pragma::Off };
                }
                b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => self.pos += 1,
                b'-' if self.peek(1) == b'-' => self.comment(),
                b'/' if self.peek(1) == b'*' && self.std >= Std::Vhdl2008 => self.block_comment(),
                b'0'..=b'9' => self.number(),
                b'"' => {
                    self.string(b'"');
                    self.push(Tok::Str, start);
                    self.check_latin1(start, self.pos, "string literal");
                }
                b'%' => {
                    // Replacement character for `"` (LRM 15.10); in 1993 (13.10) only for a
                    // string without a quotation mark (#807).
                    self.string(b'%');
                    if self.std == Std::Vhdl1993 && self.b[start..self.pos].contains(&b'"') {
                        self.error(
                            start,
                            self.pos,
                            "a string literal in `%` brackets may not contain `\"` in VHDL-1993",
                            "",
                        );
                    }
                    self.push(Tok::Str, start);
                    self.check_latin1(start, self.pos, "string literal");
                }
                b'\\' => self.extended_ident(),
                b'\'' => self.tick(),
                b'`' => self.directive(),
                _ if c.is_ascii_alphabetic() => self.ident(),
                _ if c >= 0x80 => {
                    let ch = self.char_at(start).unwrap_or(char::REPLACEMENT_CHARACTER);
                    if is_letter(ch) {
                        self.ident();
                    } else if ch == '\u{a0}' {
                        self.pos += self.src.char_len(ch);
                    } else {
                        self.unexpected(start);
                    }
                }
                _ => self.delimiter(),
            }
        }
        let end = self.b.len();
        self.toks.push(Token {
            kind: Tok::Eof,
            start: end as u32,
            end: end as u32,
        });
    }

    /// A `--` comment. One whose text is `psl` followed by a PSL directive or declaration that
    /// ends with `;` in it or in the comments right below is a PSL pragma (#226): that text is
    /// lexed as tokens, the following comments too while its statement is open.
    fn comment(&mut self) {
        if self.pragma == Pragma::Line {
            return self.skip_line();
        }
        let mut p = self.pos + 2;
        while matches!(self.b.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        let psl = self
            .b
            .get(p..p + 3)
            .is_some_and(|w| w.eq_ignore_ascii_case(b"psl"))
            && matches!(self.b.get(p + 3), Some(b' ' | b'\t'));
        if self.psl && psl && self.is_pragma(p + 3) {
            self.pos = p + 3;
            self.pragma = Pragma::Line;
        } else if self.pragma == Pragma::Open {
            self.pos += 2;
            self.pragma = Pragma::Line;
        } else {
            self.skip_line();
        }
    }

    /// Does the comment text from `p` hold a PSL statement: an optional `label :`, a directive
    /// or declaration keyword, and a `;` before the first line that is not a comment or is
    /// another `-- psl` comment? Prose such as `-- PSL assert failed` stays a comment.
    fn is_pragma(&self, mut p: usize) -> bool {
        const STARTS: &[&str] = &[
            "default",
            "assert",
            "assume",
            "assume_guarantee",
            "cover",
            "restrict",
            "restrict_guarantee",
            "fairness",
            "property",
            "sequence",
        ];
        let word = |p: &mut usize| {
            while matches!(self.b.get(*p), Some(b' ' | b'\t')) {
                *p += 1;
            }
            let s = *p;
            while self
                .b
                .get(*p)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                *p += 1;
            }
            String::from_utf8_lossy(&self.b[s..*p]).to_ascii_lowercase()
        };
        let mut w = word(&mut p);
        let mut q = p;
        while matches!(self.b.get(q), Some(b' ' | b'\t')) {
            q += 1;
        }
        if self.b.get(q) == Some(&b':') && self.b.get(q + 1) != Some(&b'=') {
            p = q + 1;
            w = word(&mut p);
        }
        if !STARTS.contains(&w.as_str()) {
            return false;
        }
        loop {
            match self.b.get(p) {
                None => return false,
                Some(b';') => return true,
                Some(b'\n') => {
                    p += 1;
                    while matches!(self.b.get(p), Some(b' ' | b'\t' | b'\r')) {
                        p += 1;
                    }
                    if self.b.get(p..p + 2) != Some(b"--") {
                        return false;
                    }
                    p += 2;
                    while matches!(self.b.get(p), Some(b' ' | b'\t')) {
                        p += 1;
                    }
                    if self
                        .b
                        .get(p..p + 4)
                        .is_some_and(|w| w.eq_ignore_ascii_case(b"psl "))
                    {
                        return false;
                    }
                }
                _ => p += 1,
            }
        }
    }

    fn skip_line(&mut self) {
        while self.pos < self.b.len() && self.b[self.pos] != b'\n' {
            self.pos += 1;
        }
    }

    fn block_comment(&mut self) {
        let start = self.pos;
        self.pos += 2;
        while self.pos < self.b.len() {
            if self.b[self.pos] == b'*' && self.peek(1) == b'/' {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
        self.error(
            start,
            start + 2,
            "unterminated block comment",
            "comment starts here",
        );
    }

    fn delimiter(&mut self) {
        let start = self.pos;
        let (c0, c1, c2) = (self.peek(0), self.peek(1), self.peek(2));
        // `?`, `<<` and `>>` are delimiters from 2008 on (LRM 15.3); `?` is not VHDL-1993.
        let (c0, c1, c2) = if self.std < Std::Vhdl2008 {
            if c0 == b'?' {
                self.unexpected(start);
                return;
            }
            match (c0, c1) {
                (b'<' | b'>', b'<' | b'>') if c0 == c1 => (c0, 0, 0),
                _ => (c0, c1, c2),
            }
        } else {
            (c0, c1, c2)
        };
        let (kind, len) = match (c0, c1, c2) {
            (b'=', b'>', _) => (Tok::Arrow, 2),
            (b'*', b'*', _) => (Tok::Pow, 2),
            (b':', b'=', _) => (Tok::Assign, 2),
            (b'/', b'=', _) => (Tok::Ne, 2),
            (b'>', b'=', _) => (Tok::Ge, 2),
            (b'<', b'=', _) => (Tok::Le, 2),
            (b'<', b'-', b'>') => (Tok::Equiv, 3),
            (b'<', b'>', _) => (Tok::Box, 2),
            (b'<', b'<', _) => (Tok::LtLt, 2),
            (b'>', b'>', _) => (Tok::GtGt, 2),
            (b'?', b'?', _) => (Tok::Cond, 2),
            (b'?', b'/', b'=') => (Tok::MatchNe, 3),
            (b'?', b'<', b'=') => (Tok::MatchLe, 3),
            (b'?', b'>', b'=') => (Tok::MatchGe, 3),
            (b'?', b'=', _) => (Tok::MatchEq, 2),
            (b'?', b'<', _) => (Tok::MatchLt, 2),
            (b'?', b'>', _) => (Tok::MatchGt, 2),
            (b'&', ..) => (Tok::Amp, 1),
            (b'(', ..) => (Tok::LParen, 1),
            (b')', ..) => (Tok::RParen, 1),
            (b'*', ..) => (Tok::Star, 1),
            (b'+', ..) => (Tok::Plus, 1),
            (b',', ..) => (Tok::Comma, 1),
            (b'-', b'>', _) => (Tok::Imply, 2),
            (b'-', ..) => (Tok::Minus, 1),
            (b'.', ..) => (Tok::Dot, 1),
            (b'/', ..) => (Tok::Slash, 1),
            (b':', ..) => (Tok::Colon, 1),
            (b';', ..) => (Tok::Semi, 1),
            (b'<', ..) => (Tok::Lt, 1),
            (b'=', ..) => (Tok::Eq, 1),
            (b'>', ..) => (Tok::Gt, 1),
            (b'|' | b'!', ..) => (Tok::Bar, 1),
            (b'[', ..) => (Tok::LBracket, 1),
            (b']', ..) => (Tok::RBracket, 1),
            (b'?', ..) => (Tok::Question, 1),
            (b'@', ..) => (Tok::At, 1),
            (b'{', ..) => (Tok::LBrace, 1),
            (b'}', ..) => (Tok::RBrace, 1),
            (b'^', ..) => (Tok::Caret, 1),
            _ => {
                self.unexpected(start);
                return;
            }
        };
        self.pos += len;
        self.push(kind, start);
    }

    /// Scan a string literal body delimited by `q`, with doubled `q` as escape.
    /// The string of a bit string literal: its bit value keeps the underline rule.
    fn bit_value(&mut self, q: u8) {
        let open = self.pos;
        self.string(q);
        if self.pos > open + 1 && self.b[self.pos - 1] == q {
            self.check_underlines(open + 1, self.pos - 1, true, "a bit string literal");
        }
    }

    fn string(&mut self, q: u8) {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.b.get(self.pos) {
                Some(&c) if c == q => {
                    if self.peek(1) == q {
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        return;
                    }
                }
                None | Some(b'\n') | Some(b'\r') => {
                    self.error(
                        start,
                        start + 1,
                        "unterminated string literal",
                        "string starts here",
                    );
                    return;
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    fn extended_ident(&mut self) {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.b.get(self.pos) {
                Some(b'\\') if self.peek(1) == b'\\' => self.pos += 2,
                Some(b'\\') => {
                    self.pos += 1;
                    break;
                }
                None | Some(b'\n') | Some(b'\r') => {
                    self.error(
                        start,
                        start + 1,
                        "unterminated extended identifier",
                        "identifier starts here",
                    );
                    break;
                }
                Some(_) => self.pos += 1,
            }
        }
        self.push(Tok::ExtIdent, start);
        self.check_latin1(start, self.pos, "extended identifier");
    }

    fn tick(&mut self) {
        let start = self.pos;
        let after_name = matches!(
            self.toks.last().map(|t| t.kind),
            Some(Tok::Ident | Tok::ExtIdent | Tok::RParen | Tok::RBracket | Tok::Kw(Kw::All))
        );
        if !after_name && let Some(ch) = self.char_at(start + 1) {
            let close = start + 1 + self.src.char_len(ch);
            if ch != '\n' && self.b.get(close) == Some(&b'\'') {
                self.pos = close + 1;
                self.push(Tok::Char, start);
                self.check_latin1(start, self.pos, "character literal");
                return;
            }
        }
        self.pos += 1;
        self.push(Tok::Tick, start);
    }

    fn word_end(&self, mut p: usize) -> usize {
        while let Some(ch) = self.char_at(p) {
            if ch == '_' || ch.is_ascii_alphanumeric() || (!ch.is_ascii() && is_letter(ch)) {
                p += self.src.char_len(ch);
            } else {
                break;
            }
        }
        p
    }

    /// An identifier or keyword of ASCII letters, digits and `_` (the common case, no
    /// allocation); `false` when a byte above 0x7f follows it, which may continue the word.
    fn ascii_ident(&mut self, start: usize) -> bool {
        let mut p = start;
        while self
            .b
            .get(p)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            p += 1;
        }
        if self.b.get(p).is_some_and(|&c| c >= 0x80) {
            return false;
        }
        self.pos = p;
        // The longest reserved word is 17 bytes; the longest bit string base 2.
        let mut buf = [0u8; 24];
        let lower = buf.get_mut(..p - start).map(|dst| {
            dst.copy_from_slice(&self.b[start..p]);
            dst.make_ascii_lowercase();
            &*dst
        });
        let q = self.peek(0);
        if matches!(q, b'"' | b'%') && lower.is_some_and(|l| self.is_base(l)) {
            self.bit_value(q);
            self.push(Tok::BitStr, start);
            self.check_latin1(start, self.pos, "bit string literal");
            return true;
        }
        let kind = match lower.and_then(|l| Kw::lookup_bytes(l, self.std)) {
            Some(kw) => Tok::Kw(kw),
            None => Tok::Ident,
        };
        self.check_underlines(start, p, false, "an identifier");
        self.push(kind, start);
        true
    }

    fn ident(&mut self) {
        let start = self.pos;
        if self.ascii_ident(start) {
            return;
        }
        self.pos = self.word_end(start);
        let lower = self
            .src
            .text(start as u32, self.pos as u32)
            .to_ascii_lowercase();
        let q = self.peek(0);
        if matches!(q, b'"' | b'%') && self.is_base(lower.as_bytes()) {
            self.bit_value(q);
            self.push(Tok::BitStr, start);
            self.check_latin1(start, self.pos, "bit string literal");
            return;
        }
        self.check_latin1(start, self.pos, "identifier");
        self.check_underlines(start, self.pos, false, "an identifier");
        let kind = match Kw::lookup(&lower, self.std) {
            Some(kw) => Tok::Kw(kw),
            None => Tok::Ident,
        };
        self.push(kind, start);
    }

    fn digits(&mut self, based: bool) -> bool {
        let start = self.pos;
        while let Some(&c) = self.b.get(self.pos) {
            let ok = if based {
                c.is_ascii_alphanumeric()
            } else {
                c.is_ascii_digit()
            };
            if ok || (c == b'_' && self.pos > start) {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.pos > start
    }

    fn number(&mut self) {
        let start = self.pos;
        self.digits(false);
        // Sized bit string such as 8x"FF".
        let word = self.word_end(self.pos);
        if self.std >= Std::Vhdl2008
            && word > self.pos
            && matches!(self.b.get(word), Some(b'"' | b'%'))
        {
            let base = self
                .src
                .text(self.pos as u32, word as u32)
                .to_ascii_lowercase();
            if BASES.contains(&base.as_str()) {
                let q = self.b[word];
                self.pos = word;
                self.bit_value(q);
                self.push(Tok::BitStr, start);
                self.check_latin1(start, self.pos, "bit string literal");
                return;
            }
        }
        let mut ok = true;
        if matches!(self.peek(0), b'#' | b':') && self.peek(1).is_ascii_alphanumeric() {
            let delim = self.peek(0);
            let int_end = self.pos;
            self.pos += 1;
            self.digits(true);
            if self.peek(0) == b'.' {
                self.pos += 1;
                ok &= self.digits(true);
            }
            if self.peek(0) == delim {
                self.pos += 1;
            } else if delim == b':' {
                // Not a based literal after all: `:` is a delimiter.
                self.pos = int_end;
                self.push(Tok::Abstract, start);
                return;
            } else {
                ok = false;
            }
        } else if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            self.pos += 1;
            self.digits(false);
        }
        if matches!(self.peek(0), b'e' | b'E')
            && (self.peek(1).is_ascii_digit()
                || (matches!(self.peek(1), b'+' | b'-') && self.peek(2).is_ascii_digit()))
        {
            self.pos += 2;
            self.digits(false);
        }
        // A literal must be separated from a following identifier (`10ns`).
        // Common in real code, so accept it with a warning.
        let end = self.word_end(self.pos);
        if end > self.pos {
            if ok && self.char_at(self.pos).is_some_and(|c| c.is_alphabetic()) {
                self.diags.push(
                    Diagnostic::warning(
                        codes::NO_SEPARATOR,
                        "missing space between a number and an identifier",
                    )
                    .with_primary(Span::new(self.file, start as u32, end as u32), "")
                    .with_help("write a space before the unit, as in `10 ns`"),
                );
            } else {
                self.pos = end;
                ok = false;
            }
        }
        if !ok {
            self.error(start, self.pos, "malformed numeric literal", "");
        } else {
            self.check_underlines(start, self.pos, false, "a numeric literal");
        }
        self.push(Tok::Abstract, start);
    }

    fn directive(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let end = self.word_end(self.pos);
        if end == self.pos {
            self.error(
                start,
                start + 1,
                "unexpected character",
                "expected a directive name after the backtick",
            );
            return;
        }
        if protect::begins_envelope(self.b, start + 1) {
            return self.encrypted(start);
        }
        self.pos = end;
        self.push(Tok::Directive, start);
    }

    /// `` `protect begin_protected ``: the rest of the file is encoded text, not source. Report
    /// it once and stop lexing so that no token of the envelope is read.
    fn encrypted(&mut self, start: usize) {
        let (agent, keys) = protect::envelope_info(self.b, start);
        let line = &self.b[start..];
        let end = start + line.iter().position(|&b| b == b'\n').unwrap_or(line.len());
        let end = start + self.b[start..end].trim_ascii_end().len();
        let mut d = Diagnostic::error(codes::ENCRYPTED, "encrypted (IEEE 1735 protected) source")
            .with_primary(
                Span::new(self.file, start as u32, end as u32),
                "the envelope starts here and runs to the end of the file",
            )
            .with_help(
                "yavs never decrypts protected IP: analyze the vendor's unencrypted simulation \
                 model instead, or simulate this block with a tool that holds the vendor's key",
            );
        if let Some(a) = agent {
            d = d.with_note(format!(
                "encrypted by `{a}`, with {keys} vendor key block(s)"
            ));
        }
        self.diags.push(d);
        self.pos = self.b.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf8(s: &str) -> Src<'_> {
        Src {
            bytes: s.as_bytes(),
            latin1: false,
        }
    }

    fn kinds(src: &str) -> Vec<Tok> {
        let mut d = Vec::new();
        let t = lex(FileId(0), utf8(src), Std::Vhdl2008, false, &mut d);
        assert!(d.is_empty(), "{d:?}");
        t.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn tick_vs_char() {
        use Tok::*;
        assert_eq!(
            kinds("t'('1') x'length f(a)'b := '''"),
            [
                Ident, Tick, LParen, Char, RParen, Ident, Tick, Ident, Ident, LParen, Ident,
                RParen, Tick, Ident, Assign, Char, Eof
            ]
        );
    }

    #[test]
    fn literals() {
        use Tok::*;
        assert_eq!(
            kinds(r#"12_3 1.5e-3 16#FF# 2#1.1#E2 8x"FF" X"0A" ub"1" "a""b" \ext\\id\ 1E6"#),
            [
                Abstract, Abstract, Abstract, Abstract, BitStr, BitStr, BitStr, Str, ExtIdent,
                Abstract, Eof
            ]
        );
    }

    #[test]
    fn delimiters() {
        use Tok::*;
        assert_eq!(
            kinds("?/= ?<= ?? <<>> <= => := ** /* x */ -- c\n<>"),
            [
                MatchNe, MatchLe, Cond, LtLt, GtGt, Le, Arrow, Assign, Pow, Box, Eof
            ]
        );
    }

    #[test]
    fn keywords_by_std() {
        let mut d = Vec::new();
        let t = lex(
            FileId(0),
            utf8("VIEW Private"),
            Std::Vhdl2019,
            false,
            &mut d,
        );
        assert_eq!(t[0].kind, Tok::Kw(Kw::View));
        assert_eq!(t[1].kind, Tok::Kw(Kw::Private));
        assert_eq!(kinds("view")[0], Tok::Ident);
    }

    #[test]
    fn lexical_differences_of_1993() {
        let lex93 = |src: &str| {
            let mut d = Vec::new();
            let t = lex(FileId(0), utf8(src), Std::Vhdl1993, false, &mut d);
            (t.iter().map(|t| t.kind).collect::<Vec<_>>(), d.len())
        };
        // Words 2008 reserved are identifiers; the older ones stay reserved.
        let (k, n) = lex93("default context protected force xnor");
        assert_eq!(k[..4], [Tok::Ident; 4]);
        assert_eq!(k[4], Tok::Kw(Kw::Xnor));
        assert_eq!(n, 0);
        // `/*` is a divide and a multiply; bit string literals have no length or other bases.
        let (k, _) = lex93("a /* b");
        assert_eq!(k, [Tok::Ident, Tok::Slash, Tok::Star, Tok::Ident, Tok::Eof]);
        assert_eq!(
            lex93("x\"AB\" b\"01\"").0,
            [Tok::BitStr, Tok::BitStr, Tok::Eof]
        );
        assert_eq!(lex93("8x\"AB\"").0[..2], [Tok::Abstract, Tok::BitStr]);
        assert_eq!(lex93("ux\"AB\"").0[..2], [Tok::Ident, Tok::Str]);
        // `?` is not a character of the language; `<<` is two tokens.
        assert_eq!(lex93("a ?= b").1, 1);
        assert_eq!(lex93("<<").0, [Tok::Lt, Tok::Lt, Tok::Eof]);
    }

    #[test]
    fn errors() {
        let mut d = Vec::new();
        lex(
            FileId(0),
            utf8("\"abc\n 16#ab $ /* x"),
            Std::Vhdl2008,
            false,
            &mut d,
        );
        let msgs: Vec<_> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "unterminated string literal",
                "malformed numeric literal",
                "unexpected character `$` (U+0024)",
                "unterminated block comment"
            ]
        );
    }
}
