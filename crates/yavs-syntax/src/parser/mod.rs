//! Recursive-descent parser.
//!
//! Every parse function returns `PResult`. An `Err` means a diagnostic was
//! already reported and the caller must resynchronise; list parsers do that
//! (skipping to the next `;`), so one bad statement does not end the parse.
//! The parser never panics or loops: every list loop makes progress, nesting
//! is capped, and a fuel counter ends the parse if it ever stalls.

mod decl;
mod expr;
mod psl;
mod psl_named;
mod psl_prop;
mod psl_seq;
mod stmt;
mod unit;

use std::borrow::Cow;

use yavs_diag::{Diagnostic, FileId, Span};

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
    /// The current design unit already got its PSL warning.
    psl_warned: bool,
    /// Token index of the expression of the `default clock` in scope.
    psl_clock: Option<usize>,
    /// Keys of the PSL `property` and `sequence` declarations skipped so far.
    psl_names: Vec<Symbol>,
    /// Reading a boolean of a PSL sequence: a `[` opens a repetition, not a signature.
    psl_no_sig: bool,
    /// The PSL sequences and properties declared so far in this design unit.
    psl_decls: Vec<psl_named::PslDecl>,
    /// The formals of the named sequence or property being expanded.
    psl_env: psl_named::Env,
    /// Token index where the body being expanded ends: a `;` there ends a sequence.
    psl_end: usize,
    /// Boolean actuals of the expansions in this directive.
    psl_args: psl_named::Args,
    /// Expansions in this directive, and the number of the latest.
    psl_expansions: u32,
    psl_scope: u32,
    /// Lists under construction, one stack per element type: a list parser pushes its
    /// elements here and moves them into a vector of exactly their number, so no
    /// list grows (and copies its large elements) by doubling.
    sc: Scratch,
}

#[derive(Default)]
struct Scratch {
    stmts: Vec<SeqStmt>,
    conc: Vec<ConcStmt>,
    decls: Vec<Decl>,
    assoc: Vec<AssocElem>,
    iface: Vec<InterfaceDecl>,
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
        psl_warned: false,
        psl_clock: None,
        psl_names: Vec::new(),
        psl_no_sig: false,
        psl_decls: Vec::new(),
        psl_env: Vec::new(),
        psl_end: usize::MAX,
        psl_args: Vec::new(),
        psl_expansions: 0,
        psl_scope: 0,
        sc: Scratch::default(),
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

    /// `what` needs VHDL-2008 (`E0011`) in a VHDL-1993 file.
    fn require_2008(&mut self, span: Span, what: &str) {
        if self.std < Std::Vhdl2008 {
            self.report(
                Diagnostic::error(codes::NEEDS_2008, format!("{what} requires VHDL-2008"))
                    .with_primary(span, "")
                    .with_help("analyze with `--std 2008`"),
            );
        }
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

    /// Skip the PSL construct that starts at the current token, up to and
    /// including its `;` (outside parentheses, brackets and braces), and warn
    /// once per design unit, unless it is formal-only (`psl_formal_only`).
    fn psl_skip(&mut self) {
        if !self.psl_formal_only() {
            self.psl_warn();
        }
        self.sync();
    }

    /// The once-per-design-unit PSL warning at the current token.
    fn psl_warn(&mut self) {
        if !self.psl_warned {
            self.psl_warned = true;
            let span = self.span();
            self.diags.push(
                Diagnostic::warning(
                    codes::PSL_SKIPPED,
                    "PSL outside the supported subset is skipped",
                )
                .with_primary(span, "PSL starts here")
                .with_note(
                    "yavs runs only clocked `assert`/`assume` with `always`, `never`, `->`, \
                         `next[n]`, `|->` and `|=>` over booleans and braced sequences, and \
                         `cover` of a boolean or a sequence; other PSL declarations, directives \
                         and verification units are ignored, so their properties are not checked",
                ),
            );
        }
    }

    /// Is the `assert` at the current token a PSL directive? A VHDL assertion
    /// has no `{`, `next` or `strong` in it and no PSL operator word; the scan
    /// runs to the `;`. A lone `assert always;` (a signal named like an
    /// operator) stays a VHDL assertion.
    fn at_psl_directive(&mut self) -> bool {
        let last = self.eof_index();
        let mut i = self.pos + 1;
        while i < last {
            let t = self.toks[i];
            match t.kind {
                Tok::Semi | Tok::Kw(Kw::End | Kw::Begin) => return false,
                Tok::LBrace
                | Tok::Imply
                | Tok::Equiv
                | Tok::Kw(Kw::Next | Kw::Strong | Kw::Until) => {
                    return true;
                }
                Tok::Bar if matches!(self.toks[i + 1].kind, Tok::Imply | Tok::Arrow) => {
                    return true;
                }
                Tok::Ident => {
                    if !(self.psl_names.is_empty() && self.psl_decls.is_empty()) {
                        let key = self.make_ident(t).key;
                        if self.psl_names.contains(&key) || self.psl_decls.iter().any(|d| d.is(key))
                        {
                            return true;
                        }
                    }
                    let w = self.text(t).to_ascii_lowercase();
                    if matches!(w.as_str(), "nondet" | "nondet_vector")
                        && self.toks[i + 1].kind == Tok::LParen
                    {
                        return true;
                    }
                    // Operators between two operands (`a before b`): a signal that has such a
                    // name, as in `assert abort = '0'`, is not one.
                    let ends = |k: Tok| {
                        matches!(
                            k,
                            Tok::Ident
                                | Tok::ExtIdent
                                | Tok::RParen
                                | Tok::RBracket
                                | Tok::RBrace
                                | Tok::Abstract
                                | Tok::Char
                                | Tok::Str
                                | Tok::BitStr
                        )
                    };
                    let starts = |k: Tok| {
                        matches!(
                            k,
                            Tok::Ident
                                | Tok::ExtIdent
                                | Tok::LParen
                                | Tok::LBrace
                                | Tok::Abstract
                                | Tok::Char
                                | Tok::Str
                                | Tok::BitStr
                                | Tok::Kw(Kw::Not)
                        )
                    };
                    let infix = matches!(
                        w.as_str(),
                        "iff"
                            | "before"
                            | "before_"
                            | "until_"
                            | "abort"
                            | "sync_abort"
                            | "async_abort"
                    ) && i > self.pos + 1
                        && ends(self.toks[i - 1].kind)
                        && starts(self.toks[i + 1].kind);
                    if infix
                        || matches!(w.as_str(), "always" | "never" | "eventually" | "forall")
                        || w.starts_with("next_")
                    {
                        let alone = i == self.pos + 1
                            && matches!(
                                self.toks[i + 1].kind,
                                Tok::Semi | Tok::Kw(Kw::Report | Kw::Severity)
                            );
                        if !alone {
                            return true;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// Report an internal compiler error at `span`.
    fn ice(&mut self, span: Span, msg: &str) {
        self.diags.push(
            Diagnostic::error(codes::INTERNAL, format!("internal error: {msg}"))
                .with_primary(span, "while parsing this")
                .with_note("this is a bug in yavs; please report it with this file"),
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

    /// Skip to just after the next `;` at bracket (and PSL brace) depth zero, stopping
    /// before `end`, `begin` or end of file.
    fn sync(&mut self) {
        let mut depth = 0u32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth = depth.saturating_sub(1),
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
                let mut buf = [0u8; 64];
                match buf.get_mut(..s.len()) {
                    Some(dst) => {
                        dst.copy_from_slice(s.as_bytes());
                        dst.make_ascii_lowercase();
                        Symbol::intern(std::str::from_utf8(dst).expect("checked ASCII"))
                    }
                    None => Symbol::intern(&s.to_ascii_lowercase()),
                }
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
    /// The closing keywords are required unless `optional`. VHDL-2019 makes
    /// only `end component` optional (LCS-2016-055a, `lrm-checks.md` P1); every
    /// other ending keeps its 2008 rule.
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
            } else if eaten > 0 || !optional {
                let found = self.peek().describe();
                let span = self.span();
                let mut d = Diagnostic::error(
                    codes::SYNTAX,
                    format!("expected `{}`, found {found}", k.as_str()),
                )
                .with_primary(span, format!("expected `end {}`", kws_str(kws)))
                .with_secondary(open, format!("to close this {what}"));
                if eaten == 0 && self.std >= Std::Vhdl2019 {
                    d = d.with_help(format!(
                        "write `end {}`: VHDL-2019 makes only the `component` keyword \
                         optional after `end` (LCS-2016-055a)",
                        kws_str(kws)
                    ));
                }
                self.report(d);
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

impl Parser<'_> {
    /// The designator after a subprogram body's `end` repeats the subprogram's (LRM 4.3, #808).
    fn check_end_designator(&mut self, end: &Designator, name: &Designator) {
        let key = |d: &Designator| match d {
            Designator::Ident(i) => (0, i.key.as_str().to_string()),
            Designator::Operator(s, _) => (1, s.as_str().to_ascii_lowercase()),
            Designator::Char(c) => (2, c.value.to_string()),
        };
        let span_text = |d: &Designator| match d {
            Designator::Ident(i) => (i.span, i.text.as_str().to_string()),
            Designator::Operator(s, sp) => (*sp, format!("\"{}\"", s.as_str())),
            Designator::Char(c) => (c.span, format!("'{}'", c.value)),
        };
        if key(end) != key(name) {
            let ((es, et), (ns, nt)) = (span_text(end), span_text(name));
            self.report(
                Diagnostic::error(
                    codes::END_NAME,
                    format!("end designator `{et}` does not match `{nt}`"),
                )
                .with_primary(es, format!("expected `{nt}`"))
                .with_secondary(ns, "declared here"),
            );
        }
    }
}

fn kws_str(kws: &[Kw]) -> String {
    kws.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(" ")
}
