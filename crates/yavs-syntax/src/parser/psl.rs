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
//! A call of a built-in function (`prev`, `stable`, `rose`, `fell`, `isunknown`, `countones`,
//! `onehot`, `onehot0`) becomes a variable of that process, set before the check from the
//! values at the clock event (`builtins`, below). Anything else is left to the skip with `W0004`: an attempt that does not fit the subset
//! restores the position and drops its diagnostics.

use super::{PResult, Parser, Recover};
use crate::Symbol;
use crate::ast::*;
use crate::token::{Kw, Tok};
use yavs_diag::Span;

/// PSL built-in functions the checker evaluates (see `Gen`).
const HANDLED: &[&str] = &[
    "rose",
    "fell",
    "prev",
    "stable",
    "onehot",
    "onehot0",
    "countones",
    "isunknown",
];

/// PSL built-in functions outside the subset: a call of one skips the directive.
const BUILTINS: &[&str] = &["ended", "nondet", "nondet_vector"];

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
        let mut prop = if self.bump().kind == Tok::Kw(Kw::Cover) {
            let brace = self.eat(Tok::LBrace);
            let b = self.expr()?;
            if brace {
                self.expect(Tok::RBrace)?;
            }
            Prop::Cover(b)
        } else {
            self.psl_top()?
        };
        let mut bi = Gen::new(self.span_from(start));
        match &mut prop {
            Prop::Always(b) | Prop::Never(b) | Prop::Cover(b) => self.psl_builtins(b, &mut bi)?,
            Prop::Imply(a, _, b) => {
                self.psl_builtins(a, &mut bi)?;
                self.psl_builtins(b, &mut bi)?;
            }
        }
        bi.finish();
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
            bi,
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
    bi: Gen,
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
        let mut stmts = vec![wait];
        stmts.extend(bi.pre);
        stmts.push(when(b, vec![stmt(then)]));
        stmts.extend(bi.post);
        return ProcessStmt {
            sensitivity: None,
            decls: bi.decls,
            stmts,
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
    let mut decls = bi.decls;
    let mut stmts = vec![wait];
    stmts.extend(bi.pre);
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
    stmts.extend(bi.post);
    ProcessStmt {
        sensitivity: None,
        decls,
        stmts,
        end_label: None,
    }
}

/// The variables and statements the built-in calls of one directive need: `decls` go in the
/// process, `pre` runs after the clock event and before the check, `post` after it.
struct Gen {
    span: Span,
    n: u32,
    /// A `first` flag is needed (`prev` of the first tick is the value itself).
    first: bool,
    decls: Vec<Decl>,
    pre: Vec<SeqStmt>,
    post: Vec<SeqStmt>,
}

impl Gen {
    fn new(span: Span) -> Self {
        Gen {
            span,
            n: 0,
            first: false,
            decls: Vec::new(),
            pre: Vec::new(),
            post: Vec::new(),
        }
    }

    fn ident(&self, s: &str) -> Ident {
        Ident {
            text: Symbol::intern(s),
            key: Symbol::intern(s),
            span: self.span,
        }
    }

    fn name(&self, kind: NameKind) -> Name {
        Name {
            kind,
            span: self.span,
        }
    }

    fn var(&self, s: &str) -> Expr {
        self.expr(ExprKind::Name(self.name(NameKind::Simple(self.ident(s)))))
    }

    fn expr(&self, kind: ExprKind) -> Expr {
        Expr {
            kind,
            span: self.span,
        }
    }

    fn int(&self, v: &str) -> Expr {
        self.expr(ExprKind::Literal(Literal {
            kind: LiteralKind::Abstract(Symbol::intern(v)),
            span: self.span,
        }))
    }

    fn paren(&self, e: Expr) -> Expr {
        self.expr(ExprKind::Paren(Box::new(e)))
    }

    fn bin(&self, op: BinaryOp, left: Expr, right: Expr) -> Expr {
        self.expr(ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn stmt(&self, kind: SeqStmtKind) -> SeqStmt {
        SeqStmt {
            label: None,
            kind,
            span: self.span,
        }
    }

    fn assign(&self, target: &str, value: Expr) -> SeqStmt {
        self.stmt(SeqStmtKind::VariableAssign {
            target: self.var(target),
            rhs: Rhs::Simple(value),
        })
    }

    fn when(&self, cond: Expr, then: Vec<SeqStmt>) -> SeqStmt {
        self.stmt(SeqStmtKind::If {
            branches: vec![(cond, then)],
            else_: None,
            end_label: None,
        })
    }

    /// `variable <names> : <subtype> [:= default];`
    fn decl(&mut self, names: &[String], subtype: SubtypeIndication, default: Option<Expr>) {
        self.decls.push(Decl::Object(ObjectDecl {
            class: ObjectClass::Variable,
            names: names.iter().map(|n| self.ident(n)).collect(),
            subtype,
            signal_kind: None,
            default,
            private: false,
            span: self.span,
        }));
    }

    fn subtype(&self, mark: Name) -> SubtypeIndication {
        SubtypeIndication {
            resolution: None,
            type_mark: mark,
            constraint: None,
            span: self.span,
        }
    }

    fn simple_subtype(&self, s: &str) -> SubtypeIndication {
        self.subtype(self.name(NameKind::Simple(self.ident(s))))
    }

    /// `x'attr` of a name.
    fn attr(&self, x: Name, attr: &str) -> Name {
        self.name(NameKind::Attribute {
            prefix: Box::new(x),
            attr: self.ident(attr),
        })
    }

    /// `\psl first\`, declared on first use: `true` until the first tick has passed.
    fn need_first(&mut self) {
        if !self.first {
            self.first = true;
            let boolean = self.simple_subtype("boolean");
            let yes = self.var("true");
            self.decl(&["\\psl first\\".to_string()], boolean, Some(yes));
        }
    }

    /// The last statement of `post`.
    fn finish(&mut self) {
        if self.first {
            let no = self.var("false");
            self.post.push(self.assign("\\psl first\\", no));
        }
    }
}

impl Parser<'_> {
    /// Replace the built-in calls in `e` by the variables that hold their values at this tick.
    // ponytail: `prev` and `stable` need an object name (the variable takes its subtype with
    // `'subtype`); `countones` and its relatives need a one-dimensional array name; `isunknown`
    // needs a std_logic-based argument (`ieee.std_logic_1164.is_x`). Other arguments skip the
    // directive with W0004.
    fn psl_builtins(&mut self, e: &mut Expr, g: &mut Gen) -> PResult<()> {
        match &mut e.kind {
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => self.psl_builtins(x, g),
            ExprKind::Qualified { operand, .. } => self.psl_builtins(operand, g),
            ExprKind::Binary { left, right, .. } => {
                self.psl_builtins(left, g)?;
                self.psl_builtins(right, g)
            }
            ExprKind::Conditional(alts) => {
                for (v, c) in alts {
                    self.psl_builtins(v, g)?;
                    if let Some(c) = c {
                        self.psl_builtins(c, g)?;
                    }
                }
                Ok(())
            }
            ExprKind::Name(n) => {
                if let Some(r) = self.psl_call(n, g)? {
                    *e = r;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The value expression of a call of a handled built-in, `None` for any other name (whose
    /// index and call arguments are searched for built-ins).
    fn psl_call(&mut self, n: &mut Name, g: &mut Gen) -> PResult<Option<Expr>> {
        let NameKind::Apply { prefix, args } = &mut n.kind else {
            return Ok(None);
        };
        let which = match &prefix.kind {
            NameKind::Simple(id) => id.key.as_str().to_string(),
            _ => String::new(),
        };
        if !HANDLED.contains(&which.as_str()) {
            for a in args {
                if let Actual::Expr(x) = &mut a.actual {
                    self.psl_builtins(x, g)?;
                }
            }
            return Ok(None);
        }
        let mut xs: Vec<Expr> = Vec::new();
        for a in std::mem::take(args) {
            match a {
                AssocElem {
                    formal: None,
                    actual: Actual::Expr(x),
                    ..
                } => xs.push(x),
                _ => return Err(Recover),
            }
        }
        g.n += 1;
        let k = g.n;
        let var = |what: &str| format!("\\psl {which} {k} {what}\\");
        let boolean = |g: &Gen| g.simple_subtype("boolean");
        let no = |g: &Gen| g.var("false");
        Ok(Some(match (which.as_str(), xs.len()) {
            ("isunknown", 1) => {
                let call = |g: &Gen, x: Expr| {
                    let sel = |p: Name, s: &str| {
                        g.name(NameKind::Selected {
                            prefix: Box::new(p),
                            suffix: Suffix::Ident(g.ident(s)),
                        })
                    };
                    let ieee = g.name(NameKind::Simple(g.ident("ieee")));
                    let lib = sel(sel(ieee, "std_logic_1164"), "is_x");
                    g.expr(ExprKind::Name(g.name(NameKind::Apply {
                        prefix: Box::new(lib),
                        args: vec![AssocElem {
                            formal: None,
                            actual: Actual::Expr(x),
                            span: g.span,
                        }],
                    })))
                };
                call(g, xs.remove(0))
            }
            ("rose" | "fell", 1) => {
                g.need_first();
                let (ct, cf, pt, pf) = (var("ct"), var("cf"), var("pt"), var("pf"));
                let b = boolean(g);
                g.decl(
                    &[ct.clone(), cf.clone(), pt.clone(), pf.clone()],
                    b,
                    Some(no(g)),
                );
                let x = xs.remove(0);
                let x2 = self.psl_reparse(&x)?;
                let yes = g.var("true");
                let cond_t = g.paren(x);
                let not_x = g.expr(ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(g.paren(x2)),
                });
                let set_t = g.when(cond_t, vec![g.assign(&ct, yes)]);
                let yes = g.var("true");
                let set_f = g.when(not_x, vec![g.assign(&cf, yes)]);
                g.pre.push(g.assign(&ct, no(g)));
                g.pre.push(set_t);
                g.pre.push(g.assign(&cf, no(g)));
                g.pre.push(set_f);
                let init = g.when(
                    g.var("\\psl first\\"),
                    vec![g.assign(&pt, g.var(&ct)), g.assign(&pf, g.var(&cf))],
                );
                g.pre.push(init);
                g.post.push(g.assign(&pt, g.var(&ct)));
                g.post.push(g.assign(&pf, g.var(&cf)));
                // rose: true now, false before; fell: false now, true before. A value that
                // is neither (an unknown `std_ulogic`) is no edge.
                let (a, b) = if which == "rose" {
                    (&ct, &pf)
                } else {
                    (&cf, &pt)
                };
                let both = g.bin(BinaryOp::And, g.var(a), g.var(b));
                g.paren(both)
            }
            ("prev" | "stable", 1 | 2) => {
                g.need_first();
                let n = match xs.get(1) {
                    None => 1,
                    Some(Expr {
                        kind:
                            ExprKind::Literal(Literal {
                                kind: LiteralKind::Abstract(v),
                                ..
                            }),
                        ..
                    }) => v
                        .as_str()
                        .parse::<u32>()
                        .ok()
                        .filter(|n| (1..=MAX_NEXT).contains(n))
                        .ok_or(Recover)?,
                    Some(_) => return Err(Recover),
                };
                if which == "stable" && xs.len() != 1 {
                    return Err(Recover);
                }
                let x = xs.remove(0);
                let ExprKind::Name(xname) = x.kind else {
                    return Err(Recover);
                };
                let obj = |p: &mut Self| -> PResult<Name> {
                    match p.psl_reparse_at(xname.span.start)?.kind {
                        ExprKind::Name(n) => Ok(n),
                        _ => Err(Recover),
                    }
                };
                let names: Vec<String> = (1..=n).map(|j| var(&format!("{j}"))).collect();
                let sub = g.subtype(g.attr(obj(self)?, "subtype"));
                g.decl(&names, sub, None);
                let cur = var("cur");
                let sub = g.subtype(g.attr(obj(self)?, "subtype"));
                g.decl(std::slice::from_ref(&cur), sub, None);
                let now = self.psl_reparse_at(xname.span.start)?;
                g.pre.push(g.stmt(SeqStmtKind::VariableAssign {
                    target: g.var(&cur),
                    rhs: Rhs::Simple(now),
                }));
                let init = g.when(
                    g.var("\\psl first\\"),
                    names.iter().map(|p| g.assign(p, g.var(&cur))).collect(),
                );
                g.pre.push(init);
                for j in (1..n as usize).rev() {
                    g.post.push(g.assign(&names[j], g.var(&names[j - 1])));
                }
                g.post.push(g.assign(&names[0], g.var(&cur)));
                let prev = g.var(&names[n as usize - 1]);
                if which == "prev" {
                    prev
                } else {
                    let eq = g.bin(BinaryOp::Eq, g.var(&cur), prev);
                    g.paren(eq)
                }
            }
            ("countones" | "onehot" | "onehot0", 1) => {
                let cnt = var("n");
                g.decl(
                    std::slice::from_ref(&cnt),
                    g.simple_subtype("natural"),
                    Some(g.int("0")),
                );
                let x = xs.remove(0);
                let ExprKind::Name(xname) = x.kind else {
                    return Err(Recover);
                };
                let at = xname.span.start;
                let elem = |p: &mut Self, g: &Gen| -> PResult<Expr> {
                    let ExprKind::Name(v) = p.psl_reparse_at(at)?.kind else {
                        return Err(Recover);
                    };
                    Ok(g.expr(ExprKind::Name(g.name(NameKind::Apply {
                        prefix: Box::new(v),
                        args: vec![AssocElem {
                            formal: None,
                            actual: Actual::Expr(g.var("\\psl i\\")),
                            span: g.span,
                        }],
                    }))))
                };
                let range = g.attr(xname, "range");
                let bump = g.assign(&cnt, g.bin(BinaryOp::Add, g.var(&cnt), g.int("1")));
                g.pre.push(g.assign(&cnt, g.int("0")));
                let test = elem(self, g)?;
                g.pre.push(g.stmt(SeqStmtKind::Loop {
                    scheme: Some(LoopScheme::For(
                        g.ident("\\psl i\\"),
                        DiscreteRange::Range(Range::Name(range)),
                    )),
                    stmts: vec![g.when(test, vec![bump])],
                    end_label: None,
                }));
                match which.as_str() {
                    "countones" => g.var(&cnt),
                    "onehot" => g.paren(g.bin(BinaryOp::Eq, g.var(&cnt), g.int("1"))),
                    _ => g.paren(g.bin(BinaryOp::Le, g.var(&cnt), g.int("1"))),
                }
            }
            _ => return Err(Recover),
        }))
    }

    /// The expression that starts at byte offset `start`, parsed again (the tree is not
    /// `Clone`).
    fn psl_reparse_at(&mut self, start: u32) -> PResult<Expr> {
        let i = self.toks.partition_point(|t| t.start < start);
        if self.toks.get(i).is_none_or(|t| t.start != start) {
            return Err(Recover);
        }
        let back = self.pos;
        self.pos = i;
        let e = self.expr();
        self.pos = back;
        e
    }

    fn psl_reparse(&mut self, e: &Expr) -> PResult<Expr> {
        self.psl_reparse_at(e.span.start)
    }
}
