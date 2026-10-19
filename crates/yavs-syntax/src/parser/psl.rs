//! The PSL subset that runs (#226). A directive of the subset becomes an implicit checker
//! process in the tree, so analysis, lowering and the kernel see an ordinary process:
//!
//! ```text
//! [l:] assert always a -> next[n] b @c report r severity s;
//! [l:] process
//!   variable \psl next 1\ .. \psl next n\ : boolean := false;
//! begin
//!   wait until c;
//!   if \psl next n\ then assert b report r severity s; end if;
//!   \psl next n\ := \psl next n-1\; .. \psl next 2\ := \psl next 1\;
//!   if a then \psl next 1\ := true; else \psl next 1\ := false; end if;
//! end process;
//! ```
//!
//! Anything else is left to the skip with `W0004`: an attempt that does not fit the subset
//! restores the position and drops its diagnostics.

use super::{PResult, Parser, Recover};
use crate::Symbol;
use crate::ast::*;
use crate::token::{Kw, Tok};
use yavs_diag::Span;

/// PSL built-in functions: a call of one is outside the subset.
const BUILTINS: &[&str] = &[
    "rose",
    "fell",
    "prev",
    "stable",
    "onehot",
    "onehot0",
    "countones",
    "isunknown",
    "ended",
    "nondet",
    "nondet_vector",
];

/// PSL operator words the VHDL expression parser would take for names.
const OPERATORS: &[&str] = &[
    "always",
    "never",
    "eventually",
    "within",
    "abort",
    "async_abort",
    "sync_abort",
    "union",
    "inf",
    "forall",
];

/// Most cycles a `next[n]` may look ahead.
// ponytail: one variable per cycle; a counter queue if longer windows are ever needed.
const MAX_NEXT: u32 = 256;

enum Prop {
    /// `always b`
    Always(Expr),
    /// `never b`
    Never(Expr),
    /// `always a -> next[n] b`; `n = 0` for `a -> b`.
    Imply(Expr, u32, Expr),
    /// `cover {b}`
    Cover(Expr),
}

type Save = (usize, usize, Option<u32>);

impl Parser<'_> {
    fn psl_save(&self) -> Save {
        (self.pos, self.diags.len(), self.last_error)
    }

    fn psl_restore(&mut self, (pos, n, last): Save) {
        self.pos = pos;
        self.diags.truncate(n);
        self.last_error = last;
    }

    /// `default clock is c;` at the current `default`: remembered for the directives of this
    /// region. `false`, with nothing consumed, for any other form.
    pub(super) fn psl_default_clock(&mut self) -> bool {
        let save = self.psl_save();
        let clock = self.nth(1) == Tok::Ident
            && self
                .text(self.toks[self.pos + 1])
                .eq_ignore_ascii_case("clock")
            && self.nth(2) == Tok::Kw(Kw::Is);
        if clock {
            self.pos += 3;
            let start = self.pos;
            if self.expr().is_ok() && self.at(Tok::Semi) && self.diags.len() == save.1 {
                self.psl_clock = Some(start);
                self.bump();
                return true;
            }
        }
        self.psl_restore(save);
        false
    }

    /// Remember the name of a skipped `property` or `sequence` declaration: a directive naming
    /// it is outside the subset.
    pub(super) fn psl_skipped_name(&mut self) {
        if self.nth(1) == Tok::Ident {
            let id = self.make_ident(self.toks[self.pos + 1]);
            self.psl_names.push(id.key);
        }
    }

    /// The `assert`, `assume` or `cover` directive at the current token as its checker
    /// process, or `None`, with nothing consumed or reported, outside the subset.
    pub(super) fn psl_directive(&mut self) -> Option<ConcStmtKind> {
        let save = self.psl_save();
        match self.psl_directive_inner() {
            Ok(p) if self.diags.len() == save.1 => Some(ConcStmtKind::Process(p)),
            _ => {
                self.psl_restore(save);
                None
            }
        }
    }

    fn psl_directive_inner(&mut self) -> PResult<ProcessStmt> {
        if self.psl_foreign() {
            return Err(Recover);
        }
        let start = self.span();
        let prop = if self.bump().kind == Tok::Kw(Kw::Cover) {
            let brace = self.eat(Tok::LBrace);
            let b = self.expr()?;
            if brace {
                self.expect(Tok::RBrace)?;
            }
            Prop::Cover(b)
        } else {
            self.psl_top()?
        };
        let clock = if self.eat(Tok::At) {
            self.expr()?
        } else {
            let from = self.psl_clock.ok_or(Recover)?;
            let back = self.pos;
            self.pos = from;
            let c = self.expr();
            self.pos = back;
            c?
        };
        let report = if self.eat_kw(Kw::Report) {
            Some(self.expr()?)
        } else {
            None
        };
        let severity = if self.eat_kw(Kw::Severity) {
            Some(self.expr()?)
        } else {
            None
        };
        self.expect(Tok::Semi)?;
        Ok(checker(
            prop,
            clock,
            report,
            severity,
            self.span_from(start),
        ))
    }

    /// Does the directive name a PSL built-in, operator word (other than its leading
    /// `always`/`never`) or skipped declaration?
    fn psl_foreign(&self) -> bool {
        let mut depth = 0u32;
        let mut i = self.pos + 1;
        while i < self.eof_index() {
            let t = self.toks[i];
            match t.kind {
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth = depth.saturating_sub(1),
                Tok::Semi if depth == 0 => return false,
                Tok::Ident => {
                    let w = self.text(t).to_ascii_lowercase();
                    let call = self.toks[i + 1].kind == Tok::LParen;
                    let directive = |k: Tok| matches!(k, Tok::Kw(Kw::Assert | Kw::Assume));
                    let lead = directive(self.toks[i - 1].kind)
                        || self.toks[i - 1].kind == Tok::LParen && directive(self.toks[i - 2].kind);
                    if (call && BUILTINS.contains(&w.as_str()))
                        || (!lead && OPERATORS.contains(&w.as_str()))
                        || w.starts_with("next_")
                        || w.starts_with("before")
                        || w.starts_with("until")
                        || self.psl_names.contains(&self.make_ident(t).key)
                    {
                        return true;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// The lowercase text of the identifier `n` tokens ahead.
    fn psl_word(&self, n: usize) -> Option<String> {
        (self.nth(n) == Tok::Ident).then(|| {
            self.text(self.toks[(self.pos + n).min(self.eof_index())])
                .to_ascii_lowercase()
        })
    }

    /// `always p`, `never b`, either in parentheses.
    fn psl_top(&mut self) -> PResult<Prop> {
        if self.at(Tok::LParen) && matches!(self.psl_word(1).as_deref(), Some("always" | "never")) {
            self.bump();
            let p = self.nested(|p| p.psl_top())?;
            self.expect(Tok::RParen)?;
            return Ok(p);
        }
        match self.psl_word(0).as_deref() {
            Some("always") => {
                self.bump();
                self.psl_prop()
            }
            Some("never") => {
                self.bump();
                Ok(Prop::Never(self.expr()?))
            }
            _ => Err(Recover),
        }
    }

    /// `b`, `a -> b`, `a -> next b`, `a -> next[n] b`, a property in parentheses.
    fn psl_prop(&mut self) -> PResult<Prop> {
        if self.at(Tok::LParen) && self.paren_has_imply() {
            self.bump();
            let p = self.nested(|p| p.psl_prop())?;
            self.expect(Tok::RParen)?;
            return Ok(p);
        }
        let a = self.expr()?;
        if !self.eat(Tok::Imply) {
            return Ok(Prop::Always(a));
        }
        let mut n = 0;
        if self.eat_kw(Kw::Next) {
            n = 1;
            if self.eat(Tok::LBracket) {
                if self.peek() != Tok::Abstract {
                    return Err(Recover);
                }
                let t = self.bump();
                n = self
                    .text(t)
                    .parse::<u32>()
                    .ok()
                    .filter(|&n| n <= MAX_NEXT)
                    .ok_or(Recover)?;
                self.expect(Tok::RBracket)?;
            }
        }
        Ok(Prop::Imply(a, n, self.expr()?))
    }

    /// Is there a `->` directly inside the parentheses that open at the current token?
    fn paren_has_imply(&self) -> bool {
        let mut depth = 0u32;
        for t in &self.toks[self.pos..] {
            match t.kind {
                Tok::LParen => depth += 1,
                Tok::RParen if depth <= 1 => return false,
                Tok::RParen => depth -= 1,
                Tok::Imply if depth == 1 => return true,
                Tok::Semi | Tok::Eof => return false,
                _ => {}
            }
        }
        false
    }
}

/// The checker process of one directive; every node it adds takes the directive's span.
fn checker(
    prop: Prop,
    clock: Expr,
    report: Option<Expr>,
    severity: Option<Expr>,
    span: Span,
) -> ProcessStmt {
    let ident = |s: &str| Ident {
        text: Symbol::intern(s),
        key: Symbol::intern(s),
        span,
    };
    let name = |s: &str| Expr {
        kind: ExprKind::Name(Name {
            kind: NameKind::Simple(ident(s)),
            span,
        }),
        span,
    };
    let stmt = |kind| SeqStmt {
        label: None,
        kind,
        span,
    };
    let when = |cond, then| {
        stmt(SeqStmtKind::If {
            branches: vec![(cond, then)],
            else_: None,
            end_label: None,
        })
    };
    let pending = |k: u32| format!("\\psl next {k}\\");
    let set = |k: u32, value: Expr| {
        stmt(SeqStmtKind::VariableAssign {
            target: name(&pending(k)),
            rhs: Rhs::Simple(value),
        })
    };
    let wait = stmt(SeqStmtKind::Wait {
        on: None,
        until: Some(clock),
        timeout: None,
    });
    if let Prop::Cover(b) = prop {
        // A hit prints the `report` text, as a note by default; without one, nothing.
        let then = match report {
            Some(report) => SeqStmtKind::Report {
                report,
                severity: Some(severity.unwrap_or_else(|| name("note"))),
            },
            None => SeqStmtKind::Null,
        };
        return ProcessStmt {
            sensitivity: None,
            decls: Vec::new(),
            stmts: vec![wait, when(b, vec![stmt(then)])],
            end_label: None,
        };
    }
    let severity = Some(severity.unwrap_or_else(|| name("error")));
    let report = Some(report.unwrap_or_else(|| Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::String(Symbol::intern("PSL assertion failed")),
            span,
        }),
        span,
    }));
    let check = |cond| {
        stmt(SeqStmtKind::Assert(Assertion {
            cond,
            report,
            severity,
        }))
    };
    let mut decls = Vec::new();
    let mut stmts = vec![wait];
    match prop {
        Prop::Always(b) => stmts.push(check(b)),
        Prop::Never(b) => stmts.push(when(b, vec![check(name("false"))])),
        Prop::Imply(a, 0, b) => stmts.push(when(a, vec![check(b)])),
        Prop::Imply(a, n, b) => {
            decls.push(Decl::Object(ObjectDecl {
                class: ObjectClass::Variable,
                names: (1..=n).map(|k| ident(&pending(k))).collect(),
                subtype: SubtypeIndication {
                    resolution: None,
                    type_mark: Name {
                        kind: NameKind::Simple(ident("boolean")),
                        span,
                    },
                    constraint: None,
                    span,
                },
                signal_kind: None,
                default: Some(name("false")),
                private: false,
                span,
            }));
            stmts.push(when(name(&pending(n)), vec![check(b)]));
            for k in (2..=n).rev() {
                stmts.push(set(k, name(&pending(k - 1))));
            }
            stmts.push(stmt(SeqStmtKind::If {
                branches: vec![(a, vec![set(1, name("true"))])],
                else_: Some(vec![set(1, name("false"))]),
                end_label: None,
            }));
        }
        Prop::Cover(_) => {} // built above
    }
    ProcessStmt {
        sensitivity: None,
        decls,
        stmts,
        end_label: None,
    }
}
