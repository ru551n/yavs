//! Hand-written lexer. Produces every token of a file, ending with `Eof`.

use rsim_diag::{Diagnostic, FileId, Span};

use crate::Std;
use crate::codes;
use crate::token::{Kw, Tok, Token};

pub fn lex(file: FileId, src: &str, std: Std, diags: &mut Vec<Diagnostic>) -> Vec<Token> {
    let mut lx = Lexer {
        file,
        src,
        b: src.as_bytes(),
        pos: 0,
        std,
        diags,
        toks: Vec::with_capacity(src.len() / 4),
    };
    lx.run();
    lx.toks
}

struct Lexer<'a> {
    file: FileId,
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    std: Std,
    diags: &'a mut Vec<Diagnostic>,
    toks: Vec<Token>,
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

    fn char_at(&self, pos: usize) -> Option<char> {
        self.src.get(pos..).and_then(|s| s.chars().next())
    }

    fn error(&mut self, start: usize, end: usize, msg: &str, label: &str) {
        self.diags.push(
            Diagnostic::error(codes::LEX, msg)
                .with_primary(Span::new(self.file, start as u32, end as u32), label),
        );
    }

    fn push(&mut self, kind: Tok, start: usize) {
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
                b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => self.pos += 1,
                b'-' if self.peek(1) == b'-' => self.skip_line(),
                b'/' if self.peek(1) == b'*' => self.block_comment(),
                b'0'..=b'9' => self.number(),
                b'"' => {
                    self.string(b'"');
                    self.push(Tok::Str, start);
                }
                b'%' => {
                    // Replacement character for `"` (LRM 15.10).
                    self.string(b'%');
                    self.push(Tok::Str, start);
                }
                b'\\' => self.extended_ident(),
                b'\'' => self.tick(),
                b'`' => self.directive(),
                _ if c.is_ascii_alphabetic() => self.ident(),
                _ if c >= 0x80 => {
                    let ch = self.char_at(start).unwrap_or('\u{fffd}');
                    if is_letter(ch) {
                        self.ident();
                    } else if ch == '\u{a0}' {
                        self.pos += ch.len_utf8();
                    } else {
                        self.pos += ch.len_utf8();
                        self.error(start, self.pos, "unexpected character", "not valid in VHDL");
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
        let (kind, len) = match (c0, c1, c2) {
            (b'=', b'>', _) => (Tok::Arrow, 2),
            (b'*', b'*', _) => (Tok::Pow, 2),
            (b':', b'=', _) => (Tok::Assign, 2),
            (b'/', b'=', _) => (Tok::Ne, 2),
            (b'>', b'=', _) => (Tok::Ge, 2),
            (b'<', b'=', _) => (Tok::Le, 2),
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
                self.pos += 1;
                self.error(start, self.pos, "unexpected character", "not valid in VHDL");
                return;
            }
        };
        self.pos += len;
        self.push(kind, start);
    }

    /// Scan a string literal body delimited by `q`, with doubled `q` as escape.
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
    }

    fn tick(&mut self) {
        let start = self.pos;
        let after_name = matches!(
            self.toks.last().map(|t| t.kind),
            Some(Tok::Ident | Tok::ExtIdent | Tok::RParen | Tok::RBracket | Tok::Kw(Kw::All))
        );
        if !after_name && let Some(ch) = self.char_at(start + 1) {
            let close = start + 1 + ch.len_utf8();
            if ch != '\n' && self.b.get(close) == Some(&b'\'') {
                self.pos = close + 1;
                self.push(Tok::Char, start);
                return;
            }
        }
        self.pos += 1;
        self.push(Tok::Tick, start);
    }

    fn word_end(&self, mut p: usize) -> usize {
        while let Some(ch) = self.char_at(p) {
            if ch == '_' || ch.is_ascii_alphanumeric() || (!ch.is_ascii() && is_letter(ch)) {
                p += ch.len_utf8();
            } else {
                break;
            }
        }
        p
    }

    fn ident(&mut self) {
        let start = self.pos;
        self.pos = self.word_end(start);
        let text = &self.src[start..self.pos];
        let lower = text.to_ascii_lowercase();
        let q = self.peek(0);
        if matches!(q, b'"' | b'%') && BASES.contains(&lower.as_str()) {
            self.string(q);
            self.push(Tok::BitStr, start);
            return;
        }
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
        if word > self.pos && matches!(self.b.get(word), Some(b'"' | b'%')) {
            let base = self.src[self.pos..word].to_ascii_lowercase();
            if BASES.contains(&base.as_str()) {
                let q = self.b[word];
                self.pos = word;
                self.string(q);
                self.push(Tok::BitStr, start);
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
        self.pos = end;
        self.push(Tok::Directive, start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        let mut d = Vec::new();
        let t = lex(FileId(0), src, Std::Vhdl2008, &mut d);
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
        let t = lex(FileId(0), "VIEW Private", Std::Vhdl2019, &mut d);
        assert_eq!(t[0].kind, Tok::Kw(Kw::View));
        assert_eq!(t[1].kind, Tok::Kw(Kw::Private));
        assert_eq!(kinds("view")[0], Tok::Ident);
    }

    #[test]
    fn errors() {
        let mut d = Vec::new();
        lex(FileId(0), "\"abc\n 16#ab $ /* x", Std::Vhdl2008, &mut d);
        let msgs: Vec<_> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "unterminated string literal",
                "malformed numeric literal",
                "unexpected character",
                "unterminated block comment"
            ]
        );
    }
}
