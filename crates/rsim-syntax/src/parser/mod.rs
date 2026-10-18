//! Recursive-descent parser.
//!
//! Every parse function returns `PResult`. An `Err` means a diagnostic was
//! already reported and the caller must resynchronise; list parsers do that
//! (skipping to the next `;`), so one bad statement does not end the parse.
//! The parser never panics or loops: every list loop makes progress, nesting
//! is capped, and a fuel counter ends the parse if it ever stalls.

mod decl;
mod expr;
mod stmt;
mod unit;

use std::borrow::Cow;

use rsim_diag::{Diagnostic, FileId, Span};

use crate::ast::*;
use crate::source::Src;
use crate::token::{Kw, Tok, Token};
use crate::{Std, Symbol, codes};

/// A diagnostic has been reported; resynchronise.
pub(crate) struct Recover;

pub(crate) type PResult<T> = Result<T, Recover>;

/// Deepest nesting of expressions, statements and declarations.
const MAX_DEPTH: u32 = 64;

pub(crate) struct Parser<'a> {
    file: FileId,
    src: Src<'a>,
    toks: Vec<Token>,
    pos: usize,
    std: Std,
    diags: &'a mut Vec<Diagnostic>,
    /// Start offset of the last token an error was reported at.
    last_error: Option<u32>,
    /// Start offsets of lexical errors: syntax errors there are cascades.
    lex_errors: Vec<u32>,
    depth: u32,
    fuel: u64,
}

pub(crate) fn parse(
    file: FileId,
    src: Src,
    toks: Vec<Token>,
    std: Std,
    diags: &mut Vec<Diagnostic>,
) -> DesignFile {
    let fuel = 200 * toks.len() as u64 + 10_000;
    let lex_errors = diags
        .iter()
        .filter(|d| d.code == codes::LEX)
        .filter_map(|d| d.primary.as_ref().map(|l| l.span.start))
        .collect();
    let mut p = Parser {
        file,
        src,
        toks,
        pos: 0,
        std,
        diags,
        last_error: None,
        lex_errors,
        depth: 0,
        fuel,
    };
    p.design_file()
}

impl<'a> Parser<'a> {
    // Token access.

    fn eof_index(&self) -> usize {
        self.toks.len() - 1
    }

    fn peek(&mut self) -> Tok {
        if self.fuel == 0 {
            return Tok::Eof;
        }
        self.fuel -= 1;
        if self.fuel == 0 {
            let span = self.span();
            self.ice(span, "the parser stopped making progress");
            self.pos = self.eof_index();
            return Tok::Eof;
        }
        self.toks[self.pos].kind
    }

    fn nth(&self, n: usize) -> Tok {
        self.toks[(self.pos + n).min(self.eof_index())].kind
    }

    fn at(&mut self, t: Tok) -> bool {
        self.peek() == t
    }

    fn at_kw(&mut self, k: Kw) -> bool {
        self.peek() == Tok::Kw(k)
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos];
        if self.pos < self.eof_index() {
            self.pos += 1;
        }
        t
    }

    fn bump_span(&mut self) -> Span {
        let t = self.bump();
        self.tok_span(t)
    }

    fn eat(&mut self, t: Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Kw) -> bool {
        self.eat(Tok::Kw(k))
    }

    fn tok_span(&self, t: Token) -> Span {
        Span::new(self.file, t.start, t.end)
    }

    /// Span of the current token.
    fn span(&self) -> Span {
        self.tok_span(self.toks[self.pos])
    }

    /// Span of the previous token.
    fn prev_span(&self) -> Span {
        self.tok_span(self.toks[self.pos.saturating_sub(1)])
    }

    /// From `start` to the end of the previous token.
    fn span_from(&self, start: Span) -> Span {
        let end = self.toks[self.pos.saturating_sub(1)].end.max(start.start);
        Span::new(self.file, start.start, end)
    }

    fn text(&self, t: Token) -> Cow<'a, str> {
        self.src.text(t.start, t.end)
    }

    // Diagnostics.

    fn report(&mut self, d: Diagnostic) {
        let at = d.primary.as_ref().map(|l| l.span.start);
        if at.is_some() && at == self.last_error || at.is_some_and(|a| self.lex_errors.contains(&a))
        {
            return;
        }
        self.last_error = at;
        self.diags.push(d);
    }

    fn error(&mut self, span: Span, msg: impl Into<String>, label: impl Into<String>) {
        self.report(Diagnostic::error(codes::SYNTAX, msg).with_primary(span, label));
    }

    /// Report "expected `what`, found ..." at the current token.
    fn expected(&mut self, what: &str) -> Recover {
        let found = self.peek().describe();
        let span = self.span();
        self.error(
            span,
            format!("expected {what}, found {found}"),
            format!("expected {what}"),
        );
        Recover
    }

    fn expect(&mut self, t: Tok) -> PResult<Span> {
        if self.at(t) {
            Ok(self.bump_span())
        } else {
            Err(self.expected(&t.describe()))
        }
    }

    fn expect_kw(&mut self, k: Kw) -> PResult<Span> {
        self.expect(Tok::Kw(k))
    }

    /// Expect a `;`. If it is missing and the next token is on a new line,
    /// report it after the previous token and carry on as if it were there.
    fn semi(&mut self) -> PResult<()> {
        if self.eat(Tok::Semi) {
            return Ok(());
        }
        let prev = self.toks[self.pos.saturating_sub(1)];
        let cur = self.toks[self.pos];
        if self.src.has_newline(prev.end, cur.start) {
            let found = self.peek().describe();
            let span = self.tok_span(prev);
            self.report(
                Diagnostic::error(codes::SYNTAX, format!("expected `;`, found {found}"))
                    .with_primary(span, "expected `;` after this"),
            );
            return Ok(());
        }
        Err(self.expected("`;`"))
    }

    fn require_2019(&mut self, span: Span, what: &str) {
        if self.std < Std::Vhdl2019 {
            self.report(
                Diagnostic::error(codes::NEEDS_2019, format!("{what} requires VHDL-2019"))
                    .with_primary(span, "")
                    .with_help("analyze with `--std 2019`"),
            );
        }
    }

    fn psl_unsupported(&mut self, span: Span) {
        self.report(
            Diagnostic::error(codes::PSL, "PSL is not supported")
                .with_primary(span, "PSL starts here")
                .with_note("rsim does not implement PSL (IEEE 1850); this construct is skipped"),
        );
    }

    /// Report an internal compiler error at `span`.
    fn ice(&mut self, span: Span, msg: &str) {
        self.diags.push(
            Diagnostic::error(codes::INTERNAL, format!("internal error: {msg}"))
                .with_primary(span, "while parsing this")
                .with_note("this is a bug in rsim; please report it with this file"),
        );
    }

    /// The character of a character literal token at `span`.
    fn char_value(&mut self, span: Span) -> PResult<char> {
        match self.src.text(span.start, span.end).chars().nth(1) {
            Some(c) => Ok(c),
            None => {
                self.ice(span, "character literal without a character");
                Err(Recover)
            }
        }
    }

    /// Run `f` one nesting level deeper, failing if nesting is too deep.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        if self.depth >= MAX_DEPTH {
            let span = self.span();
            self.report(
                Diagnostic::error(codes::TOO_DEEP, "nesting is too deep")
                    .with_primary(span, format!("more than {MAX_DEPTH} levels deep")),
            );
            return Err(Recover);
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    // Recovery.

    /// Skip to just after the next `;` at bracket depth zero, stopping
    /// before `end`, `begin` or end of file.
    fn sync(&mut self) {
        let mut depth = 0u32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBracket => depth = depth.saturating_sub(1),
                Tok::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                Tok::Kw(Kw::End | Kw::Begin) if depth == 0 => return,
                _ => {}
            }
            self.bump();
        }
    }

    /// Skip up to (not past) one of `stops` at bracket depth zero, or a `;`.
    fn sync_to(&mut self, stops: &[Tok]) {
        let mut depth = 0u32;
        loop {
            let t = self.peek();
            if depth == 0 && (stops.contains(&t) || t == Tok::Semi) || t == Tok::Eof {
                return;
            }
            match t {
                Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBracket => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
        }
    }

    /// Index of the token just after the bracket matching the `(` at `pos`.
    fn after_matching_paren(&self, mut pos: usize) -> usize {
        let mut depth = 0u32;
        while pos < self.eof_index() {
            match self.toks[pos].kind {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return pos + 1;
                    }
                }
                _ => {}
            }
            pos += 1;
        }
        pos
    }

    // Identifiers and ends.

    fn make_ident(&self, t: Token) -> Ident {
        let s = self.text(t);
        let text = Symbol::intern(&s);
        let key = if t.kind == Tok::ExtIdent {
            text
        } else if s.is_ascii() {
            if s.bytes().any(|b| b.is_ascii_uppercase()) {
                Symbol::intern(&s.to_ascii_lowercase())
            } else {
                text
            }
        } else {
            Symbol::intern(&s.to_lowercase())
        };
        Ident {
            text,
            key,
            span: self.tok_span(t),
        }
    }

    fn at_ident(&mut self) -> bool {
        matches!(self.peek(), Tok::Ident | Tok::ExtIdent)
    }

    fn ident(&mut self) -> PResult<Ident> {
        if self.at_ident() {
            let t = self.bump();
            Ok(self.make_ident(t))
        } else {
            Err(self.expected("identifier"))
        }
    }

    fn ident_list(&mut self) -> PResult<Vec<Ident>> {
        let mut v = vec![self.ident()?];
        while self.eat(Tok::Comma) {
            v.push(self.ident()?);
        }
        Ok(v)
    }

    /// `label :` at the current position.
    fn label(&mut self) -> Option<Ident> {
        if self.at_ident() && self.nth(1) == Tok::Colon {
            let t = self.bump();
            self.bump();
            Some(self.make_ident(t))
        } else {
            None
        }
    }

    /// Parse `end [kws] [name] ;` closing a construct opened at `open`.
    ///
    /// The closing keywords are required unless `optional`; VHDL-2019 makes
    /// them optional everywhere.
    fn end(
        &mut self,
        open: Span,
        what: &str,
        kws: &[Kw],
        optional: bool,
        name: Option<&Ident>,
    ) -> PResult<Option<Ident>> {
        if !self.at_kw(Kw::End) {
            let found = self.peek().describe();
            let span = self.span();
            let mut d = Diagnostic::error(codes::SYNTAX, format!("expected `end`, found {found}"))
                .with_primary(span, "expected `end`");
            if self.at(Tok::Eof) {
                d = d.with_secondary(open, format!("this {what} is never closed"));
            }
            self.report(d);
            return Err(Recover);
        }
        self.bump();
        let mut eaten = 0;
        for &k in kws {
            if self.eat_kw(k) {
                eaten += 1;
            } else if eaten > 0 || !(optional || self.std >= Std::Vhdl2019) {
                let found = self.peek().describe();
                let span = self.span();
                self.report(
                    Diagnostic::error(
                        codes::SYNTAX,
                        format!("expected `{}`, found {found}", k.as_str()),
                    )
                    .with_primary(span, format!("expected `end {}`", kws_str(kws)))
                    .with_secondary(open, format!("to close this {what}")),
                );
                break;
            } else {
                break;
            }
        }
        let end_name = if self.at_ident() {
            let t = self.bump();
            Some(self.make_ident(t))
        } else {
            None
        };
        self.check_end_label(end_name.as_ref(), name);
        self.semi()?;
        Ok(end_name)
    }
}

impl Parser<'_> {
    /// Check that an end label repeats the construct's name or label.
    fn check_end_label(&mut self, end_name: Option<&Ident>, name: Option<&Ident>) {
        match (end_name, name) {
            (Some(e), Some(n)) if e.key != n.key => {
                self.report(
                    Diagnostic::error(
                        codes::END_NAME,
                        format!("end label `{}` does not match `{}`", e.text, n.text),
                    )
                    .with_primary(e.span, format!("expected `{}`", n.text))
                    .with_secondary(n.span, "declared here"),
                );
            }
            (Some(e), None) => {
                self.error(
                    e.span,
                    format!("unexpected end label `{}`", e.text),
                    "this construct has no label",
                );
            }
            _ => {}
        }
    }
}

fn kws_str(kws: &[Kw]) -> String {
    kws.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(" ")
}
