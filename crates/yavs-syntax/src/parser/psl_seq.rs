//! PSL sequences (SEREs) in the checker subset (#445). A directive over braced sequences
//! becomes an implicit checker process, like the simple forms in `psl.rs`:
//!
//! ```text
//! cover {a; b[*2]};             -- a note at every tick a match ends
//! assert never {a; b};          -- fails at every tick a match ends
//! assert always {a} |=> {b; c}; -- every match of the left side starts an obligation
//! ```
//!
//! A sequence becomes a position automaton (Glushkov): one position per boolean in the
//! sequence (copies for a repetition count), one process variable per position, true when
//! a match in progress has just consumed that position. The left side of `|->` and `|=>`
//! and the sequence of `cover` and `never` need only "does a match end now", which the
//! union of all matches in progress answers. The right side is an obligation that fails
//! when it can no longer match, which must be told apart per start: its automaton is made
//! deterministic at parse time (subset construction over the minterms of its booleans), and
//! the process keeps one variable per reachable state. Instances in the same state behave
//! alike and merge.

use super::psl::{Gen, Prop};
use super::psl_named::DeclKind;
use super::{PResult, Parser, Recover};
use crate::ast::*;
use crate::token::{Kw, Tok};
use std::collections::HashMap;

/// Most positions in one automaton and in one deterministic obligation.
const MAX_POS: usize = 256;
/// Most distinct booleans an obligation may test (its automaton is built over their minterms).
const MAX_LEAVES: usize = 6;
/// Most states of a deterministic obligation.
const MAX_STATES: usize = 64;
/// Largest repetition count.
const MAX_COUNT: u32 = 64;

/// A sequence over numbered booleans (`SeqProp::leaves`).
pub(super) enum Sere {
    Leaf(usize),
    Cat(Vec<Sere>),
    Fuse(Vec<Sere>),
    Or(Vec<Sere>),
    /// `&&`: both match and end together.
    Len(Box<Sere>, Box<Sere>),
    /// `&`: both match, either may be longer.
    Amp(Box<Sere>, Box<Sere>),
    Within(Box<Sere>, Box<Sere>),
    /// `s[*lo to hi]`, `hi` open for `None`.
    Star(Box<Sere>, u32, Option<u32>),
    /// `b[->lo to hi]`
    Goto(usize, u32, Option<u32>),
    /// `b[=lo to hi]`
    Eq(usize, u32, Option<u32>),
}

pub(super) enum SeqKind {
    /// `cover {s}`
    Cover(Sere),
    /// `assert never {s}`
    Never(Sere),
    /// `[always] {l} |-> {r}`, `[always] {r}` (no `l`)
    Suffix {
        always: bool,
        lhs: Option<Sere>,
        overlap: bool,
        rhs: Sere,
    },
}

/// The booleans of a directive: one entry per distinct text, so a boolean used twice is
/// evaluated once.
#[derive(Default)]
pub(super) struct Leaves {
    pub exprs: Vec<Expr>,
    texts: Vec<String>,
}

pub(super) struct SeqProp {
    pub kind: SeqKind,
    pub leaves: Leaves,
}

impl Leaves {
    fn add(&mut self, e: Expr, text: String) -> Sere {
        let k = self
            .texts
            .iter()
            .position(|t| *t == text)
            .unwrap_or_else(|| {
                self.texts.push(text);
                self.exprs.push(e);
                self.exprs.len() - 1
            });
        Sere::Leaf(k)
    }
}

impl SeqProp {
    /// The simple form, when the property is one that `psl.rs` checks without an automaton.
    pub(super) fn into_simple(mut self) -> Result<Prop, SeqProp> {
        let n = self.leaves.exprs.len();
        let one = |s: &Sere, k: usize| matches!(s, Sere::Leaf(i) if *i == k);
        let simple = match &self.kind {
            SeqKind::Cover(s) if n == 1 && one(s, 0) => 0,
            SeqKind::Never(s) if n == 1 && one(s, 0) => 1,
            SeqKind::Suffix {
                always: true,
                lhs: None,
                rhs,
                ..
            } if n == 1 && one(rhs, 0) => 2,
            SeqKind::Suffix {
                always: true,
                lhs: Some(l),
                overlap,
                rhs,
            } if n == 2 && one(l, 0) && one(rhs, 1) => 3 + usize::from(!*overlap),
            _ => return Err(self),
        };
        let mut v = std::mem::take(&mut self.leaves.exprs);
        let b = if v.len() == 2 { v.pop() } else { None };
        match (simple, v.pop(), b) {
            (0, Some(a), _) => Ok(Prop::Cover(a)),
            (1, Some(a), _) => Ok(Prop::Never(a)),
            (2, Some(a), _) => Ok(Prop::Always(a)),
            (3, Some(a), Some(b)) => Ok(Prop::Imply(a, 0, b)),
            (4, Some(a), Some(b)) => Ok(Prop::Imply(a, 1, b)),
            _ => Err(self),
        }
    }

    pub(super) fn needs_first(&self) -> bool {
        matches!(self.kind, SeqKind::Suffix { always: false, .. })
    }
}

impl Parser<'_> {
    /// Does the property that starts `i` tokens ahead (`i` is 1 after the keyword of a
    /// directive, 0 in the body of a declared property) start with a sequence, possibly
    /// behind `always`, `never` or parentheses?
    pub(super) fn psl_at_sequence_from(&self, mut i: usize) -> bool {
        loop {
            match self.nth(i) {
                Tok::LParen => i += 1,
                Tok::Ident if matches!(self.psl_word(i).as_deref(), Some("always" | "never")) => {
                    i += 1
                }
                _ => return self.psl_sequence_at(i),
            }
        }
    }

    /// Is the token `i` ahead the start of a sequence: a brace, the name of a declared
    /// sequence or a sequence formal?
    pub(super) fn psl_sequence_at(&self, i: usize) -> bool {
        self.nth(i) == Tok::LBrace
            || self.psl_named_at(i, DeclKind::Sequence).is_some()
            || self.psl_formal_is_sequence(i)
            || self.psl_bare_repeat_at(i)
    }

    /// A boolean with a repetition and no braces (`a[*3]`, `(a = '1')[->2]`) starts at `i`
    /// tokens ahead: a `[` followed by `*`, `+`, `=` or `->` at depth zero before the end of
    /// the property.
    pub(super) fn psl_bare_repeat_at(&self, i: usize) -> bool {
        let mut depth = 0u32;
        let mut j = (self.pos + i).min(self.eof_index());
        while j < self.eof_index() {
            match self.toks[j].kind {
                Tok::LParen | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                Tok::LBracket if depth == 0 => {
                    return matches!(
                        self.toks[j + 1].kind,
                        Tok::Star | Tok::Plus | Tok::Eq | Tok::Imply
                    );
                }
                Tok::Semi
                | Tok::At
                | Tok::Bar
                | Tok::Imply
                | Tok::Kw(Kw::Report | Kw::Severity)
                    if depth == 0 =>
                {
                    return false;
                }
                _ => {}
            }
            j += 1;
        }
        false
    }

    /// The property of a directive whose property starts with a sequence.
    pub(super) fn psl_seq_prop(&mut self, cover: bool) -> PResult<SeqProp> {
        let mut leaves = Leaves::default();
        let paren = self.at(Tok::LParen) && !self.psl_sequence_at(1);
        if paren {
            self.bump();
        }
        let kind = if cover {
            let s = self.sere_primary(&mut leaves)?;
            SeqKind::Cover(s)
        } else {
            let word = self.psl_word(0);
            match word.as_deref() {
                Some("never") => {
                    self.bump();
                    SeqKind::Never(self.sere_primary(&mut leaves)?)
                }
                Some("always") => {
                    self.bump();
                    self.psl_suffix(true, &mut leaves)?
                }
                _ => self.psl_suffix(false, &mut leaves)?,
            }
        };
        if paren {
            self.expect(Tok::RParen)?;
        }
        Ok(SeqProp { kind, leaves })
    }

    /// `{l} |-> r`, `{l} |=> r` or `{r}`.
    fn psl_suffix(&mut self, always: bool, leaves: &mut Leaves) -> PResult<SeqKind> {
        let paren = self.at(Tok::LParen) && self.psl_sequence_at(1);
        if paren {
            self.bump();
        }
        let first = self.sere_primary(leaves)?;
        let kind = if self.at(Tok::Bar) && matches!(self.nth(1), Tok::Imply | Tok::Arrow) {
            self.bump();
            let overlap = self.bump().kind == Tok::Imply;
            let rhs = self.psl_rhs(leaves, !overlap)?;
            SeqKind::Suffix {
                always,
                lhs: Some(first),
                overlap,
                rhs,
            }
        } else {
            SeqKind::Suffix {
                always,
                lhs: None,
                overlap: true,
                rhs: first,
            }
        };
        if paren {
            self.expect(Tok::RParen)?;
        }
        Ok(kind)
    }

    /// The right side of a suffix implication: `{s}`, a boolean, either behind `next` or
    /// `next[n]`. `lead` is a tick the `|=>` form adds before it (none for `|->`): it is
    /// part of the operator, so the caller passes `false` for `|->`.
    fn psl_rhs(&mut self, leaves: &mut Leaves, _next_form: bool) -> PResult<Sere> {
        let mut lead = 0;
        if self.eat_kw(Kw::Next) {
            lead = 1;
            if self.eat(Tok::LBracket) {
                lead = self.sere_count()?;
                self.expect(Tok::RBracket)?;
            }
        }
        let body = if self.psl_sequence_at(0) {
            self.sere_primary(leaves)?
        } else {
            self.sere_leaf(leaves)?
        };
        if lead == 0 {
            return Ok(body);
        }
        let t = self.sere_true(leaves);
        Ok(Sere::Cat(vec![
            Sere::Star(Box::new(t), lead, Some(lead)),
            body,
        ]))
    }

    fn sere_true(&self, leaves: &mut Leaves) -> Sere {
        let span = self.span();
        let e = Expr {
            kind: ExprKind::Name(Name {
                kind: NameKind::Simple(Ident {
                    text: crate::Symbol::intern("true"),
                    key: crate::Symbol::intern("true"),
                    span,
                }),
                span,
            }),
            span,
        };
        leaves.add(e, "true".to_string())
    }

    /// A braced sequence, a named one or a `sequence` formal, with repetition operators.
    pub(super) fn sere_primary(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        if self.at(Tok::LBrace) {
            return self.sere_braced(leaves);
        }
        if let Some(s) = self.psl_formal_sequence(leaves)? {
            return self.sere_postfix(s, leaves);
        }
        if let Some(d) = self.psl_named(DeclKind::Sequence) {
            let s = self.psl_expand(d, |p| p.sere_concat(leaves))?;
            return self.sere_postfix(s, leaves);
        }
        if self.psl_bare_repeat_at(0) {
            let leaf = self.sere_leaf(leaves)?;
            return self.sere_postfix(leaf, leaves);
        }
        Err(Recover)
    }

    /// `{ sere }`, possibly followed by repetition operators.
    fn sere_braced(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        self.expect(Tok::LBrace)?;
        let s = self.nested(|p| p.sere_concat(leaves))?;
        self.expect(Tok::RBrace)?;
        self.sere_postfix(s, leaves)
    }

    /// `a ; b ; c`
    pub(super) fn sere_concat(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        let mut parts = vec![self.sere_fuse(leaves)?];
        while self.pos < self.psl_end && self.eat(Tok::Semi) {
            parts.push(self.sere_fuse(leaves)?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Sere::Cat(parts)
        })
    }

    /// `a : b : c`
    fn sere_fuse(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        let mut parts = vec![self.sere_compound(leaves)?];
        while self.eat(Tok::Colon) {
            parts.push(self.sere_compound(leaves)?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Sere::Fuse(parts)
        })
    }

    /// Items joined by one of `|`, `&&`, `&`, `within`; mixing them needs braces.
    fn sere_compound(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        let mut acc = self.sere_item(leaves)?;
        let mut op: Option<&str> = None;
        loop {
            let this = match self.peek() {
                Tok::Bar => "|",
                Tok::Amp if self.nth(1) == Tok::Amp => "&&",
                Tok::Amp => "&",
                _ if self.psl_word(0).as_deref() == Some("within") => "within",
                _ => return Ok(acc),
            };
            if op.is_some_and(|o| o != this) {
                return Err(Recover);
            }
            op = Some(this);
            self.bump();
            if this == "&&" {
                self.bump();
            }
            let rhs = self.sere_item(leaves)?;
            acc = match (this, acc) {
                ("|", Sere::Or(mut v)) => {
                    v.push(rhs);
                    Sere::Or(v)
                }
                ("|", a) => Sere::Or(vec![a, rhs]),
                ("&&", a) => Sere::Len(Box::new(a), Box::new(rhs)),
                ("&", a) => Sere::Amp(Box::new(a), Box::new(rhs)),
                (_, a) => Sere::Within(Box::new(a), Box::new(rhs)),
            };
        }
    }

    /// A braced sequence or a boolean, with repetition operators.
    fn sere_item(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        // `[*3]` alone is `true[*3]`.
        if self.at(Tok::LBracket) {
            let t = self.sere_true(leaves);
            return self.sere_postfix(t, leaves);
        }
        if self.psl_sequence_at(0) {
            return self.sere_primary(leaves);
        }
        let leaf = self.sere_leaf(leaves)?;
        self.sere_postfix(leaf, leaves)
    }

    /// A boolean expression. A `[` ends it: it opens a repetition, not a signature.
    pub(super) fn sere_leaf(&mut self, leaves: &mut Leaves) -> PResult<Sere> {
        let was = std::mem::replace(&mut self.psl_no_sig, true);
        let e = self.psl_expr();
        self.psl_no_sig = was;
        let e = e?;
        let text = self
            .src
            .text(e.span.start, e.span.end)
            .to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // The same text means another boolean where a formal stands for another actual.
        let key = format!("{text}#{}", self.psl_env_key());
        Ok(leaves.add(e, key))
    }

    /// `[*]`, `[*n]`, `[*m to n]`, `[+]`, `[=n]`, `[->n]` after an item.
    fn sere_postfix(&mut self, mut s: Sere, _leaves: &mut Leaves) -> PResult<Sere> {
        while self.at(Tok::LBracket) {
            self.bump();
            let kind = self.bump().kind;
            let (lo, hi) = match kind {
                Tok::Plus => (1, None),
                Tok::Star if self.at(Tok::RBracket) => (0, None),
                Tok::Imply if self.at(Tok::RBracket) => (1, Some(1)),
                Tok::Star | Tok::Eq | Tok::Imply => self.sere_range()?,
                _ => return Err(Recover),
            };
            self.expect(Tok::RBracket)?;
            s = match (kind, s) {
                (Tok::Star | Tok::Plus, s) => Sere::Star(Box::new(s), lo, hi),
                (Tok::Imply, Sere::Leaf(k)) => Sere::Goto(k, lo, hi),
                (Tok::Eq, Sere::Leaf(k)) => Sere::Eq(k, lo, hi),
                _ => return Err(Recover),
            };
        }
        Ok(s)
    }

    /// `n`, `m to n`, `m to inf`
    fn sere_range(&mut self) -> PResult<(u32, Option<u32>)> {
        let lo = self.sere_count()?;
        if !self.eat_kw(Kw::To) {
            return Ok((lo, Some(lo)));
        }
        if self.psl_word(0).as_deref() == Some("inf") {
            self.bump();
            return Ok((lo, None));
        }
        let hi = self.sere_count()?;
        if hi < lo {
            return Err(Recover);
        }
        Ok((lo, Some(hi)))
    }

    fn sere_count(&mut self) -> PResult<u32> {
        self.psl_count()
            .ok()
            .filter(|&n| n <= MAX_COUNT)
            .ok_or(Recover)
    }
}

/// A literal condition: leaf number and polarity.
pub(super) type Lit = (usize, bool);

/// A set of automaton positions with its connections (Glushkov): a fragment of the
/// automaton the positions of which live in `Nfa::conds`.
#[derive(Clone, Default)]
pub(super) struct Frag {
    pub(super) pos: Vec<usize>,
    pub(super) first: Vec<usize>,
    pub(super) last: Vec<usize>,
    pub(super) follow: Vec<(usize, usize)>,
    pub(super) nullable: bool,
}

/// The position automaton: `conds[p]` is what the tick that consumes position `p` must satisfy.
#[derive(Default)]
pub(super) struct Nfa {
    pub(super) conds: Vec<Vec<Lit>>,
}

fn union(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut v = a.to_vec();
    for x in b {
        if !v.contains(x) {
            v.push(*x);
        }
    }
    v
}

impl Nfa {
    fn leaf(&mut self, cond: Vec<Lit>) -> PResult<Frag> {
        if self.conds.len() >= MAX_POS {
            return Err(Recover);
        }
        self.conds.push(cond);
        let p = self.conds.len() - 1;
        Ok(Frag {
            pos: vec![p],
            first: vec![p],
            last: vec![p],
            follow: Vec::new(),
            nullable: false,
        })
    }

    fn empty() -> Frag {
        Frag {
            nullable: true,
            ..Frag::default()
        }
    }

    /// The fragment again, on new positions.
    fn copy(&mut self, f: &Frag) -> PResult<Frag> {
        if self.conds.len() + f.pos.len() > MAX_POS {
            return Err(Recover);
        }
        let base = self.conds.len();
        let map = |p: usize| base + f.pos.iter().position(|&q| q == p).unwrap_or(0);
        for &p in &f.pos {
            let c = self.conds[p].clone();
            self.conds.push(c);
        }
        Ok(Frag {
            pos: f.pos.iter().map(|&p| map(p)).collect(),
            first: f.first.iter().map(|&p| map(p)).collect(),
            last: f.last.iter().map(|&p| map(p)).collect(),
            follow: f.follow.iter().map(|&(p, q)| (map(p), map(q))).collect(),
            nullable: f.nullable,
        })
    }

    fn cat(&mut self, a: Frag, b: Frag) -> Frag {
        let mut follow = union_pairs(&a.follow, &b.follow);
        for &p in &a.last {
            for &q in &b.first {
                if !follow.contains(&(p, q)) {
                    follow.push((p, q));
                }
            }
        }
        Frag {
            pos: union(&a.pos, &b.pos),
            first: if a.nullable {
                union(&a.first, &b.first)
            } else {
                a.first
            },
            last: if b.nullable {
                union(&a.last, &b.last)
            } else {
                b.last
            },
            follow,
            nullable: a.nullable && b.nullable,
        }
    }

    fn or(&mut self, a: Frag, b: Frag) -> Frag {
        Frag {
            pos: union(&a.pos, &b.pos),
            first: union(&a.first, &b.first),
            last: union(&a.last, &b.last),
            follow: union_pairs(&a.follow, &b.follow),
            nullable: a.nullable || b.nullable,
        }
    }

    /// `f[*]`
    fn star(&mut self, mut f: Frag) -> Frag {
        for &p in &f.last {
            for &q in &f.first {
                if !f.follow.contains(&(p, q)) {
                    f.follow.push((p, q));
                }
            }
        }
        f.nullable = true;
        f
    }

    /// `f[*lo to hi]`
    fn rep(&mut self, f: &Frag, lo: u32, hi: Option<u32>) -> PResult<Frag> {
        let mut acc = Self::empty();
        match hi {
            None => {
                // f[*lo to inf] = f^lo ; f[*]
                for _ in 0..lo {
                    let c = self.copy(f)?;
                    acc = self.cat(acc, c);
                }
                let c = self.copy(f)?;
                let s = self.star(c);
                Ok(self.cat(acc, s))
            }
            Some(hi) => {
                for _ in 0..lo {
                    let c = self.copy(f)?;
                    acc = self.cat(acc, c);
                }
                // (f (f (f)?)?)?
                let mut opt = Self::empty();
                for _ in lo..hi {
                    let c = self.copy(f)?;
                    let mut inner = self.cat(c, opt);
                    inner.nullable = true;
                    opt = inner;
                }
                Ok(self.cat(acc, opt))
            }
        }
    }

    /// `a : b`, the last tick of `a` is the first of `b`.
    fn fuse(&mut self, a: Frag, b: Frag) -> PResult<Frag> {
        // An empty match has no last tick to overlap: only the non-empty matches fuse.
        let mut out = Frag {
            pos: union(&a.pos, &b.pos),
            first: a.first.clone(),
            last: b.last.clone(),
            follow: union_pairs(&a.follow, &b.follow),
            nullable: false,
        };
        for &p in &a.last {
            for &q in &b.first {
                let mut cond = self.conds[p].clone();
                for l in &self.conds[q] {
                    if !cond.contains(l) {
                        cond.push(*l);
                    }
                }
                if self.conds.len() >= MAX_POS {
                    return Err(Recover);
                }
                self.conds.push(cond);
                let m = self.conds.len() - 1;
                out.pos.push(m);
                for &(x, y) in &a.follow {
                    if y == p {
                        out.follow.push((x, m));
                    }
                }
                for &(x, y) in &b.follow {
                    if x == q {
                        out.follow.push((m, y));
                    }
                }
                if a.first.contains(&p) {
                    out.first.push(m);
                }
                if b.last.contains(&q) {
                    out.last.push(m);
                }
            }
        }
        // p and q stay: p still starts paths that continue inside `a`, q is reached only
        // through the merged positions.
        Ok(out)
    }

    /// `a && b`: both match, from the same tick to the same tick (product automaton).
    fn len_and(&mut self, a: &Frag, b: &Frag) -> PResult<Frag> {
        let mut id: HashMap<(usize, usize), usize> = HashMap::new();
        let mut out = Frag {
            nullable: a.nullable && b.nullable,
            ..Frag::default()
        };
        let mut todo: Vec<(usize, usize)> = Vec::new();
        for &p in &a.first {
            for &q in &b.first {
                let m = self.pair(&mut id, &mut out, &mut todo, (a, b), (p, q))?;
                out.first.push(m);
            }
        }
        let mut done = 0;
        while done < todo.len() {
            let (p, q) = todo[done];
            done += 1;
            let m = id[&(p, q)];
            for &(_, y) in a.follow.iter().filter(|e| e.0 == p) {
                for &(_, y2) in b.follow.iter().filter(|e| e.0 == q) {
                    let n = self.pair(&mut id, &mut out, &mut todo, (a, b), (y, y2))?;
                    out.follow.push((m, n));
                }
            }
        }
        Ok(out)
    }

    /// The product position of `p` of `a` and `q` of `b`, made on first use.
    fn pair(
        &mut self,
        id: &mut HashMap<(usize, usize), usize>,
        out: &mut Frag,
        todo: &mut Vec<(usize, usize)>,
        (a, b): (&Frag, &Frag),
        (p, q): (usize, usize),
    ) -> PResult<usize> {
        if let Some(&m) = id.get(&(p, q)) {
            return Ok(m);
        }
        if self.conds.len() >= MAX_POS {
            return Err(Recover);
        }
        let mut cond = self.conds[p].clone();
        for l in &self.conds[q] {
            if !cond.contains(l) {
                cond.push(*l);
            }
        }
        self.conds.push(cond);
        let m = self.conds.len() - 1;
        id.insert((p, q), m);
        out.pos.push(m);
        if a.last.contains(&p) && b.last.contains(&q) {
            out.last.push(m);
        }
        todo.push((p, q));
        Ok(m)
    }

    fn build(&mut self, s: &Sere) -> PResult<Frag> {
        match s {
            Sere::Leaf(k) => self.leaf(vec![(*k, true)]),
            Sere::Cat(v) => {
                let mut acc = self.build(&v[0])?;
                for x in &v[1..] {
                    let b = self.build(x)?;
                    acc = self.cat(acc, b);
                }
                Ok(acc)
            }
            Sere::Fuse(v) => {
                let mut acc = self.build(&v[0])?;
                for x in &v[1..] {
                    let b = self.build(x)?;
                    acc = self.fuse(acc, b)?;
                }
                Ok(acc)
            }
            Sere::Or(v) => {
                let mut acc = self.build(&v[0])?;
                for x in &v[1..] {
                    let b = self.build(x)?;
                    acc = self.or(acc, b);
                }
                Ok(acc)
            }
            Sere::Len(a, b) => {
                let (a, b) = (self.build(a)?, self.build(b)?);
                self.len_and(&a, &b)
            }
            Sere::Amp(a, b) => {
                // {a && b;true[*]} | {a;true[*] && b}
                let (a1, b1) = (self.build(a)?, self.build(b)?);
                let (a2, b2) = (self.build(a)?, self.build(b)?);
                let t = self.any()?;
                let b1t = self.cat(b1, t);
                let first = self.len_and(&a1, &b1t)?;
                let t = self.any()?;
                let a2t = self.cat(a2, t);
                let second = self.len_and(&a2t, &b2)?;
                Ok(self.or(first, second))
            }
            Sere::Within(a, b) => {
                // {true[*]; a; true[*]} && b
                let (a, b) = (self.build(a)?, self.build(b)?);
                let (t1, t2) = (self.any()?, self.any()?);
                let ta = self.cat(t1, a);
                let tat = self.cat(ta, t2);
                self.len_and(&tat, &b)
            }
            Sere::Star(inner, lo, hi) => {
                let f = self.build(inner)?;
                self.rep(&f, *lo, *hi)
            }
            Sere::Goto(k, lo, hi) | Sere::Eq(k, lo, hi) => {
                // {(!b)[*]; b}[*lo to hi], then (!b)[*] for [=]
                let skip = self.leaf(vec![(*k, false)])?;
                let skip = self.star(skip);
                let hit = self.leaf(vec![(*k, true)])?;
                let one = self.cat(skip, hit);
                let r = self.rep(&one, *lo, *hi)?;
                if matches!(s, Sere::Goto(..)) {
                    return Ok(r);
                }
                let tail = self.leaf(vec![(*k, false)])?;
                let tail = self.star(tail);
                Ok(self.cat(r, tail))
            }
        }
    }

    /// The sequence with only the positions that lie on a path from a first to a last one:
    /// a dead end would keep an obligation alive that can no longer match.
    pub(super) fn trimmed(&mut self, s: &Sere) -> PResult<Frag> {
        let mut f = self.build(s)?;
        let mut live: Vec<usize> = f.first.clone();
        let mut i = 0;
        while i < live.len() {
            let p = live[i];
            for e in f.follow.iter().filter(|e| e.0 == p) {
                if !live.contains(&e.1) {
                    live.push(e.1);
                }
            }
            i += 1;
        }
        let mut good: Vec<usize> = f
            .last
            .iter()
            .copied()
            .filter(|p| live.contains(p))
            .collect();
        let mut i = 0;
        while i < good.len() {
            let p = good[i];
            for e in f.follow.iter().filter(|e| e.1 == p) {
                if live.contains(&e.0) && !good.contains(&e.0) {
                    good.push(e.0);
                }
            }
            i += 1;
        }
        f.pos.retain(|p| good.contains(p));
        f.first.retain(|p| good.contains(p));
        f.last.retain(|p| good.contains(p));
        f.follow
            .retain(|e| good.contains(&e.0) && good.contains(&e.1));
        Ok(f)
    }

    /// `true[*]`
    fn any(&mut self) -> PResult<Frag> {
        let t = self.leaf(Vec::new())?;
        Ok(self.star(t))
    }
}

fn union_pairs(a: &[(usize, usize)], b: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut v = a.to_vec();
    for x in b {
        if !v.contains(x) {
            v.push(*x);
        }
    }
    v
}

// Code generation.

/// `\psl <what> <id>\`
pub(super) fn vname(what: &str, id: usize) -> String {
    format!("\\psl {what} {id}\\")
}

pub(super) fn not(g: &Gen, e: Expr) -> Expr {
    g.expr(ExprKind::Unary {
        op: UnaryOp::Not,
        operand: Box::new(g.paren(e)),
    })
}

/// `a or b or ..`, `false` for none.
pub(super) fn any_of(g: &Gen, mut v: Vec<Expr>) -> Expr {
    if v.is_empty() {
        return g.var("false");
    }
    let mut acc = v.remove(0);
    for e in v {
        acc = g.bin(BinaryOp::Or, acc, e);
    }
    g.paren(acc)
}

/// `a and b and ..`, `true` for none.
pub(super) fn all_of(g: &Gen, mut v: Vec<Expr>) -> Expr {
    if v.is_empty() {
        return g.var("true");
    }
    let mut acc = v.remove(0);
    for e in v {
        acc = g.bin(BinaryOp::And, acc, e);
    }
    g.paren(acc)
}

/// The conjunction of literals over the leaf variables.
pub(super) fn cond_expr(g: &Gen, cond: &[Lit]) -> Expr {
    let lits = cond
        .iter()
        .map(|&(k, pol)| {
            let v = g.var(&vname("b", k));
            if pol { v } else { not(g, v) }
        })
        .collect();
    all_of(g, lits)
}

/// Declare boolean variables.
pub(super) fn bools(g: &mut Gen, names: &[String]) {
    let t = g.simple_subtype("boolean");
    let no = g.var("false");
    g.decl(names, t, Some(no));
}

/// The automaton of a sequence as process code: `stmts` set the position variables for this
/// tick and `done` to whether a match ends now. `start` says whether a match may begin now.
pub(super) fn trigger(
    g: &mut Gen,
    nfa: &Nfa,
    f: &Frag,
    id: usize,
    start: impl Fn(&Gen) -> Expr,
    done: &str,
    stmts: &mut Vec<SeqStmt>,
) {
    let s = |p: usize| vname(&format!("s{id}"), p);
    let n = |p: usize| vname(&format!("n{id}"), p);
    let names: Vec<String> = f.pos.iter().flat_map(|&p| [s(p), n(p)]).collect();
    bools(g, &names);
    for &p in &f.pos {
        let mut before: Vec<Expr> = Vec::new();
        if f.first.contains(&p) {
            before.push(start(g));
        }
        before.extend(f.follow.iter().filter(|e| e.1 == p).map(|e| g.var(&s(e.0))));
        let reach = any_of(g, before);
        let both = g.bin(BinaryOp::And, cond_expr(g, &nfa.conds[p]), reach);
        stmts.push(g.assign(&n(p), g.paren(both)));
    }
    for &p in &f.pos {
        stmts.push(g.assign(&s(p), g.var(&n(p))));
    }
    let ends = f.last.iter().map(|&p| g.var(&n(p))).collect();
    stmts.push(g.assign(done, any_of(g, ends)));
}

/// Where a deterministic obligation goes on one tick.
#[derive(Clone, PartialEq)]
enum Go {
    Match,
    Dead,
    State(Vec<usize>),
}

struct Dfa {
    /// The leaves the obligation tests.
    used: Vec<usize>,
    /// `states[i]`: sorted positions; the start is not in the list.
    states: Vec<Vec<usize>>,
    /// `go[source][minterm]`; source 0 is the start, `i + 1` is `states[i]`.
    go: Vec<Vec<Go>>,
}

fn dfa(nfa: &Nfa, f: &Frag) -> PResult<Dfa> {
    let mut used: Vec<usize> = Vec::new();
    for &p in &f.pos {
        for &(k, _) in &nfa.conds[p] {
            if !used.contains(&k) {
                used.push(k);
            }
        }
    }
    used.sort_unstable();
    if used.len() > MAX_LEAVES {
        return Err(Recover);
    }
    let minterms = 1usize << used.len();
    let holds = |p: usize, m: usize| {
        nfa.conds[p].iter().all(|&(k, pol)| {
            let bit = used
                .iter()
                .position(|&u| u == k)
                .is_some_and(|i| m >> i & 1 == 1);
            bit == pol
        })
    };
    let step = |from: Option<&Vec<usize>>, m: usize| -> Go {
        let mut next: Vec<usize> = match from {
            None => f.first.clone(),
            Some(s) => {
                let mut v: Vec<usize> = Vec::new();
                for e in f.follow.iter().filter(|e| s.contains(&e.0)) {
                    if !v.contains(&e.1) {
                        v.push(e.1);
                    }
                }
                v
            }
        };
        next.retain(|&p| holds(p, m));
        if next.iter().any(|p| f.last.contains(p)) {
            Go::Match
        } else if next.is_empty() {
            Go::Dead
        } else {
            next.sort_unstable();
            Go::State(next)
        }
    };
    let mut states: Vec<Vec<usize>> = Vec::new();
    let mut go: Vec<Vec<Go>> = Vec::new();
    let mut i = 0;
    // source 0 is the start, then each state in order of discovery
    let mut sources: Vec<Option<Vec<usize>>> = vec![None];
    while i < sources.len() {
        let row: Vec<Go> = (0..minterms)
            .map(|m| step(sources[i].as_ref(), m))
            .collect();
        for g in &row {
            if let Go::State(s) = g
                && !states.contains(s)
            {
                if states.len() >= MAX_STATES {
                    return Err(Recover);
                }
                states.push(s.clone());
                sources.push(Some(s.clone()));
            }
        }
        go.push(row);
        i += 1;
    }
    Ok(Dfa { used, states, go })
}

/// The check of a suffix implication: obligations start where `inject` is true, the
/// process variable `fail` is true at a tick on which one cannot be met any more.
fn obligation(g: &mut Gen, d: &Dfa, id: usize, inject: Expr, fail: &str, stmts: &mut Vec<SeqStmt>) {
    let live = |i: usize| vname(&format!("d{id}"), i);
    let next = |i: usize| vname(&format!("e{id}"), i);
    let names: Vec<String> = (0..d.states.len())
        .flat_map(|i| [live(i), next(i)])
        .collect();
    if !names.is_empty() {
        bools(g, &names);
    }
    let minterms = 1usize << d.used.len();
    stmts.push(g.assign(fail, g.var("false")));
    for i in 0..d.states.len() {
        stmts.push(g.assign(&next(i), g.var("false")));
    }
    let mut inject = Some(inject);
    for (src, row) in d.go.iter().enumerate() {
        // targets in order of first use
        let mut targets: Vec<Go> = Vec::new();
        for t in row {
            if *t != Go::Match && !targets.contains(t) {
                targets.push(t.clone());
            }
        }
        let mut body = Vec::new();
        for t in &targets {
            let sel: Vec<usize> = (0..minterms).filter(|&m| row[m] == *t).collect();
            let cond = if sel.len() == minterms {
                g.var("true")
            } else {
                let terms = sel
                    .iter()
                    .map(|&m| {
                        let lits: Vec<Lit> = d
                            .used
                            .iter()
                            .enumerate()
                            .map(|(bit, &k)| (k, m >> bit & 1 == 1))
                            .collect();
                        cond_expr(g, &lits)
                    })
                    .collect();
                any_of(g, terms)
            };
            let target = match t {
                Go::Dead => fail.to_string(),
                Go::State(s) => next(d.states.iter().position(|x| x == s).unwrap_or(0)),
                Go::Match => continue,
            };
            body.push(g.when(cond, vec![g.assign(&target, g.var("true"))]));
        }
        if body.is_empty() {
            continue;
        }
        let guard = if src == 0 {
            inject.take().unwrap_or_else(|| g.var("false"))
        } else {
            g.var(&live(src - 1))
        };
        stmts.push(g.when(guard, body));
    }
    for i in 0..d.states.len() {
        stmts.push(g.assign(&live(i), g.var(&next(i))));
    }
}

/// The checker process of a directive over sequences.
pub(super) fn seq_checker(
    prop: SeqProp,
    clock: Expr,
    report: Option<Expr>,
    severity: Option<Expr>,
    mut bi: Gen,
) -> PResult<ProcessStmt> {
    let SeqProp { kind, leaves } = prop;
    let mut nfa = Nfa::default();
    let mut stmts: Vec<SeqStmt> = Vec::new();
    let g = &mut bi;
    // This tick's value of every boolean.
    let lv: Vec<String> = (0..leaves.exprs.len()).map(|k| vname("b", k)).collect();
    bools(g, &lv);
    let wait = g.stmt(SeqStmtKind::Wait {
        on: None,
        until: Some(clock),
        timeout: None,
    });
    let mut evals = Vec::new();
    for (k, e) in leaves.exprs.into_iter().enumerate() {
        evals.push(g.assign(&lv[k], g.var("false")));
        evals.push(g.when(e, vec![g.assign(&lv[k], g.var("true"))]));
    }
    let always_start = |g: &Gen| g.var("true");
    let first_start = |g: &Gen| g.var("\\psl first\\");
    match kind {
        SeqKind::Cover(s) => {
            let f = nfa.trimmed(&s)?;
            bools(g, &["\\psl match\\".to_string()]);
            trigger(g, &nfa, &f, 1, always_start, "\\psl match\\", &mut stmts);
            // A hit prints the `report` text, as a note by default; without one, nothing.
            let then = match report {
                Some(report) => SeqStmtKind::Report {
                    report,
                    severity: Some(severity.unwrap_or_else(|| g.var("note"))),
                },
                None => SeqStmtKind::Null,
            };
            let hit = g.var("\\psl match\\");
            stmts.push(g.when(hit, vec![g.stmt(then)]));
        }
        SeqKind::Never(s) => {
            let f = nfa.trimmed(&s)?;
            bools(g, &["\\psl match\\".to_string()]);
            trigger(g, &nfa, &f, 1, always_start, "\\psl match\\", &mut stmts);
            let hit = g.var("\\psl match\\");
            stmts.push(fail_stmt(g, hit, report, severity));
        }
        SeqKind::Suffix {
            always,
            lhs,
            overlap,
            rhs,
        } => {
            // ponytail: an obligation that is a star from zero, `{b[*]}`, holds at once (as in the
            // reference simulators); any other empty match is ignored. The LRM text on a
            // sequence that matches the empty word was not at hand.
            let vacuous = is_star0(&rhs);
            let rf = nfa.trimmed(&rhs)?;
            let d = dfa(&nfa, &rf)?;
            bools(g, &["\\psl fail\\".to_string()]);
            let start = |g: &Gen| {
                if always {
                    always_start(g)
                } else {
                    first_start(g)
                }
            };
            let inject = match lhs {
                None => start(g),
                Some(l) => {
                    let lf = nfa.trimmed(&l)?;
                    bools(g, &["\\psl match\\".to_string()]);
                    trigger(g, &nfa, &lf, 1, start, "\\psl match\\", &mut stmts);
                    if overlap {
                        g.var("\\psl match\\")
                    } else {
                        bools(g, &["\\psl match prev\\".to_string()]);
                        g.var("\\psl match prev\\")
                    }
                }
            };
            if !vacuous {
                obligation(g, &d, 2, inject, "\\psl fail\\", &mut stmts);
            }
            if !overlap {
                let m = g.var("\\psl match\\");
                stmts.push(g.assign("\\psl match prev\\", m));
            }
            let hit = g.var("\\psl fail\\");
            stmts.push(fail_stmt(g, hit, report, severity));
        }
    }
    let mut all = vec![wait];
    all.extend(bi.pre);
    all.extend(evals);
    all.extend(stmts);
    all.extend(bi.post);
    Ok(ProcessStmt {
        sensitivity: None,
        decls: bi.decls,
        stmts: all,
        end_label: None,
    })
}

/// `s[*]`, `s[*0 to inf]` or `s[*0]`, also behind one-element groups.
fn is_star0(s: &Sere) -> bool {
    match s {
        Sere::Star(_, 0, None | Some(0)) => true,
        Sere::Cat(v) | Sere::Fuse(v) | Sere::Or(v) if v.len() == 1 => is_star0(&v[0]),
        _ => false,
    }
}

/// `if check then assert false report .. severity ..; end if;` with the defaults of a failed
/// assertion.
pub(super) fn fail_stmt(
    g: &Gen,
    check: Expr,
    report: Option<Expr>,
    severity: Option<Expr>,
) -> SeqStmt {
    let report = report.unwrap_or_else(|| {
        g.expr(ExprKind::Literal(Literal {
            kind: LiteralKind::String(crate::Symbol::intern("PSL assertion failed")),
            span: g.span,
        }))
    });
    let severity = severity.unwrap_or_else(|| g.var("error"));
    g.when(
        check,
        vec![g.stmt(SeqStmtKind::Assert(Assertion {
            cond: g.var("false"),
            report: Some(report),
            severity: Some(severity),
        }))],
    )
}
