//! Temporal PSL properties in the checker subset (#445): `next_a`, `next_e`, `next_event`
//! and relatives, `until`, `until_`, `before`, `before_`, `sync_abort`, `<->`, `iff`, nested
//! `next` and `always`, `and`/`or` of properties, and a property on the right of `|->`/`|=>`.
//! The directive becomes a checker process like the simpler ones (`psl.rs`, `psl_seq.rs`):
//!
//! ```text
//! assert always (a -> next_e[2 to 4] b);   -- fails at the tick the window closes without b
//! assert always {a; b} |=> (c until d);    -- c holds up to, not at, the first d
//! ```
//!
//! A property is turned into terms that say what must hold from the next tick on (formula
//! progression over finite traces, weak: an obligation still open at the end of the run is no
//! failure). At parse time every term reachable from the property is enumerated over the
//! values of the booleans it tests, the terms are numbered, and the process keeps one variable
//! per term: the start of a new obligation is injected at each tick (or at the first one, or
//! where the left side of an implication matches), every live term is stepped once with the
//! tick's values, and a step to `false` is a failure. Terms in the same state merge, so a
//! failure is reported once per tick however many starts fail.
//!
//! Strong operators (`eventually!`, `until!`, `next!`) stay outside: they need a check when the
//! run ends. `abort` and `async_abort` are checked at the tick like `sync_abort` (lrm-checks.md L133).

use super::psl::Gen;
use super::psl_seq::{
    Frag, Leaves, Nfa, Sere, any_of, bools, cond_expr, fail_stmt, trigger, vname,
};
use super::{PResult, Parser, Recover};
use crate::ast::*;
use crate::token::{Kw, Tok};
use std::collections::{BTreeSet, HashMap};

/// Most terms (variables of the process) of one property.
const MAX_ATOMS: usize = 64;
/// Most booleans one term may test, and most steps tried in all (terms times valuations).
const MAX_TERM_LEAVES: usize = 6;
const MAX_STEPS: usize = 4096;
/// Largest count or window bound.
const MAX_BOUND: u32 = 64;

/// A parsed property.
pub(super) enum Pt {
    /// A property that never holds (the right side of `never {s}`).
    False,
    Leaf(usize),
    /// A boolean that must be false (`never b`)
    NotLeaf(usize),
    Seq(Sere),
    And(Vec<Pt>),
    Or(Vec<Pt>),
    Imp(usize, Box<Pt>),
    Iff(usize, usize),
    Next(u32, Box<Pt>),
    NextA(u32, u32, Box<Pt>),
    NextE(u32, u32, Box<Pt>),
    /// `next_event(c)[lo to hi](p)`: all of the occurrences, or one of them.
    NextEvent {
        c: usize,
        lo: u32,
        hi: u32,
        all: bool,
        p: Box<Pt>,
    },
    Until(Box<Pt>, usize, bool),
    Before(usize, usize, bool),
    SyncAbort(Box<Pt>, usize),
    Always(Box<Pt>),
    Suffix {
        lhs: Sere,
        overlap: bool,
        rhs: Box<Pt>,
    },
}

/// `abort` and `async_abort` run like `sync_abort`: the condition is checked at the clock tick
/// only (user decision; the LRM and the references abort between ticks, #636).
fn is_abort(w: &str) -> bool {
    matches!(w, "abort" | "async_abort" | "sync_abort")
}

pub(super) struct TProp {
    pub pt: Pt,
    pub leaves: Leaves,
}

impl Parser<'_> {
    /// Do the tokens `from..to` use an operator that only the temporal parser runs?
    pub(super) fn psl_has_temporal(&self, from: usize, to: usize) -> bool {
        let to = to.min(self.eof_index());
        // Two implications, or `and`/`or` after a parenthesis: properties joined.
        let imps = (from..to)
            .filter(|&i| self.toks[i].kind == Tok::Imply)
            .count();
        let joined = (from..to.saturating_sub(1)).any(|i| {
            self.toks[i].kind == Tok::RParen
                && matches!(self.toks[i + 1].kind, Tok::Kw(Kw::And | Kw::Or))
        });
        if imps > 1 || joined && imps > 0 {
            return true;
        }
        (from..to).any(|i| {
            let t = self.toks[i];
            match t.kind {
                Tok::Kw(Kw::Until) | Tok::Equiv => true,
                Tok::Ident => {
                    let w = self.text(t).to_ascii_lowercase();
                    w.starts_with("next_")
                        || w.starts_with("before")
                        || w == "until_"
                        || is_abort(&w)
                        || w == "iff"
                }
                _ => false,
            }
        })
    }

    /// The temporal property at the current token.
    pub(super) fn psl_temporal_prop(&mut self) -> PResult<TProp> {
        let mut leaves = Leaves::default();
        let pt = self.nested(|p| p.tp_prop(&mut leaves))?;
        Ok(TProp { pt, leaves })
    }

    fn tp_word(&self) -> Option<String> {
        self.psl_word(0)
    }

    /// `a -> p`, `a <-> b`, `a iff b`, right to left.
    fn tp_prop(&mut self, lv: &mut Leaves) -> PResult<Pt> {
        let lhs = self.tp_abort(lv)?;
        if self.eat(Tok::Imply) {
            let Pt::Leaf(k) = lhs else {
                return Err(Recover);
            };
            let rhs = self.nested(|p| p.tp_prop(lv))?;
            return Ok(Pt::Imp(k, Box::new(rhs)));
        }
        if self.at(Tok::Equiv) || self.tp_word().as_deref() == Some("iff") {
            self.bump();
            let (Pt::Leaf(a), Pt::Leaf(b)) = (lhs, self.tp_abort(lv)?) else {
                return Err(Recover);
            };
            return Ok(Pt::Iff(a, b));
        }
        Ok(lhs)
    }

    fn tp_abort(&mut self, lv: &mut Leaves) -> PResult<Pt> {
        let mut p = self.tp_until(lv)?;
        while self.tp_word().as_deref().is_some_and(is_abort) {
            self.bump();
            let b = self.tp_bool(lv)?;
            p = Pt::SyncAbort(Box::new(p), b);
        }
        Ok(p)
    }

    fn tp_until(&mut self, lv: &mut Leaves) -> PResult<Pt> {
        let lhs = self.tp_unary(lv)?;
        if self.at_kw(Kw::Until) {
            self.bump();
            if self.at(Tok::Bar) {
                return Err(Recover); // until!, strong
            }
            let b = self.tp_bool(lv)?;
            return Ok(Pt::Until(Box::new(lhs), b, false));
        }
        match self.tp_word().as_deref() {
            Some("until_") => {
                self.bump();
                if self.at(Tok::Bar) {
                    return Err(Recover);
                }
                let b = self.tp_bool(lv)?;
                Ok(Pt::Until(Box::new(lhs), b, true))
            }
            Some(w @ ("before" | "before_")) => {
                let overlap = w == "before_";
                self.bump();
                if self.at(Tok::Bar) {
                    return Err(Recover);
                }
                let Pt::Leaf(a) = lhs else {
                    return Err(Recover);
                };
                let b = self.tp_bool(lv)?;
                Ok(Pt::Before(a, b, overlap))
            }
            _ => Ok(lhs),
        }
    }

    /// A boolean: the index of its leaf.
    fn tp_bool(&mut self, lv: &mut Leaves) -> PResult<usize> {
        match self.sere_leaf(lv)? {
            Sere::Leaf(k) => Ok(k),
            _ => Err(Recover),
        }
    }

    /// `[n]` or `[lo to hi]` after a `next` operator.
    fn tp_range(&mut self) -> PResult<(u32, u32)> {
        self.expect(Tok::LBracket)?;
        let lo = self.tp_bound()?;
        let hi = if self.eat_kw(Kw::To) {
            self.tp_bound()?
        } else {
            lo
        };
        self.expect(Tok::RBracket)?;
        if hi < lo {
            return Err(Recover);
        }
        Ok((lo, hi))
    }

    fn tp_bound(&mut self) -> PResult<u32> {
        self.psl_count()
            .ok()
            .filter(|&n| n <= MAX_BOUND)
            .ok_or(Recover)
    }

    /// Does the parenthesis at the current token hold a property rather than a boolean?
    fn tp_paren_is_property(&self) -> bool {
        let mut depth = 0u32;
        for i in self.pos..self.eof_index() {
            let t = self.toks[i];
            match t.kind {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                Tok::LBrace | Tok::Imply | Tok::Equiv | Tok::Kw(Kw::Next | Kw::Until) => {
                    return true;
                }
                Tok::Ident => {
                    let w = self.text(t).to_ascii_lowercase();
                    if matches!(w.as_str(), "always" | "never" | "iff" | "until_")
                        || is_abort(&w)
                        || w.starts_with("next_")
                        || w.starts_with("before")
                        || self.psl_decls.iter().any(|d| d.is(self.make_ident(t).key))
                        || self.psl_formal_is_property(i - self.pos)
                    {
                        return true;
                    }
                }
                Tok::Semi | Tok::Eof => return false,
                _ => {}
            }
        }
        false
    }

    fn tp_unary(&mut self, lv: &mut Leaves) -> PResult<Pt> {
        let p = self.tp_unary_inner(lv)?;
        // `and` and `or` of properties; between booleans the expression parser took them.
        if matches!(self.peek(), Tok::Kw(Kw::And | Kw::Or)) {
            let is_and = self.bump().kind == Tok::Kw(Kw::And);
            let rhs = self.nested(|s| s.tp_unary(lv))?;
            return Ok(if is_and {
                Pt::And(vec![p, rhs])
            } else {
                Pt::Or(vec![p, rhs])
            });
        }
        Ok(p)
    }

    fn tp_unary_inner(&mut self, lv: &mut Leaves) -> PResult<Pt> {
        if let Some(p) = self.psl_formal_property(|s| s.tp_prop(lv))? {
            return Ok(p);
        }
        if self.at_kw(Kw::Next) {
            self.bump();
            if self.at(Tok::Bar) {
                return Err(Recover); // next!, strong
            }
            let n = if self.at(Tok::LBracket) {
                let (lo, hi) = self.tp_range()?;
                if lo != hi {
                    return Err(Recover);
                }
                lo
            } else {
                1
            };
            let p = self.nested(|s| s.tp_unary(lv))?;
            return Ok(Pt::Next(n, Box::new(p)));
        }
        if self.at(Tok::LParen) {
            if self.tp_paren_is_property() {
                self.bump();
                let p = self.nested(|s| s.tp_prop(lv))?;
                self.expect(Tok::RParen)?;
                return Ok(p);
            }
            // A boolean in parentheses, followed by `and` or `or` of another property.
            let after = self.after_matching_paren(self.pos).min(self.eof_index());
            if matches!(self.toks[after].kind, Tok::Kw(Kw::And | Kw::Or)) {
                self.bump();
                let k = self.tp_bool(lv)?;
                self.expect(Tok::RParen)?;
                return Ok(Pt::Leaf(k));
            }
            return Ok(Pt::Leaf(self.tp_bool(lv)?));
        }
        if let Some(w) = self.tp_word() {
            let call = |p: &mut Self, lv: &mut Leaves| -> PResult<Box<Pt>> {
                p.expect(Tok::LParen)?;
                let x = p.nested(|s| s.tp_prop(lv))?;
                p.expect(Tok::RParen)?;
                Ok(Box::new(x))
            };
            match w.as_str() {
                "always" => {
                    self.bump();
                    let p = self.nested(|s| s.tp_prop(lv))?;
                    return Ok(Pt::Always(Box::new(p)));
                }
                "never" => {
                    self.bump();
                    if self.psl_sequence_at(0) {
                        let s = self.sere_primary(lv)?;
                        return Ok(Pt::Suffix {
                            lhs: s,
                            overlap: true,
                            rhs: Box::new(Pt::False),
                        });
                    }
                    let p = self.nested(|s| s.tp_unary(lv))?;
                    return match p {
                        Pt::Leaf(k) => Ok(Pt::NotLeaf(k)),
                        _ => Err(Recover),
                    };
                }
                "next_a" | "next_e" => {
                    self.bump();
                    if self.at(Tok::Bar) {
                        return Err(Recover);
                    }
                    let (lo, hi) = self.tp_range()?;
                    let p = call(self, lv)?;
                    return Ok(if w == "next_a" {
                        Pt::NextA(lo, hi, p)
                    } else {
                        Pt::NextE(lo, hi, p)
                    });
                }
                "next_event" | "next_event_a" | "next_event_e" => {
                    self.bump();
                    if self.at(Tok::Bar) {
                        return Err(Recover);
                    }
                    self.expect(Tok::LParen)?;
                    let c = self.tp_bool(lv)?;
                    self.expect(Tok::RParen)?;
                    let (lo, hi) = if self.at(Tok::LBracket) {
                        self.tp_range()?
                    } else {
                        (1, 1)
                    };
                    if lo == 0 || (w == "next_event" && lo != hi) {
                        return Err(Recover);
                    }
                    let p = call(self, lv)?;
                    return Ok(Pt::NextEvent {
                        c,
                        lo,
                        hi,
                        all: w != "next_event_e",
                        p,
                    });
                }
                _ => {}
            }
        }
        if self.psl_sequence_at(0) {
            let s = self.sere_primary(lv)?;
            if self.at(Tok::Bar) && matches!(self.nth(1), Tok::Imply | Tok::Arrow) {
                self.bump();
                let overlap = self.bump().kind == Tok::Imply;
                let rhs = self.nested(|p| p.tp_prop(lv))?;
                return Ok(Pt::Suffix {
                    lhs: s,
                    overlap,
                    rhs: Box::new(rhs),
                });
            }
            return Ok(Pt::Seq(s));
        }
        let k = self.tp_bool(lv)?;
        // `a |=> p`: a boolean is a sequence of one.
        if self.at(Tok::Bar) && matches!(self.nth(1), Tok::Imply | Tok::Arrow) {
            self.bump();
            let overlap = self.bump().kind == Tok::Imply;
            let rhs = self.nested(|p| p.tp_prop(lv))?;
            return Ok(Pt::Suffix {
                lhs: Sere::Leaf(k),
                overlap,
                rhs: Box::new(rhs),
            });
        }
        Ok(Pt::Leaf(k))
    }
}

/// A residual obligation: what must hold from the current tick on.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum T {
    True,
    False,
    Lit(usize, bool),
    And(Vec<T>),
    Or(Vec<T>),
    Next(u32, Box<T>),
    Always(Box<T>),
    Until(Box<T>, usize, bool),
    Before(usize, usize, bool),
    Abort(Box<T>, usize),
    NextEvent(usize, u32, Box<T>),
    /// Sequence number and the positions reached (none before the first tick).
    Seq(usize, Option<Vec<usize>>),
}

fn mk_and(xs: Vec<T>) -> T {
    let mut v: Vec<T> = Vec::new();
    for x in xs {
        match x {
            T::True => {}
            T::False => return T::False,
            T::And(ys) => v.extend(ys),
            x => v.push(x),
        }
    }
    v.sort();
    v.dedup();
    match v.len() {
        0 => T::True,
        1 => v.remove(0),
        _ => T::And(v),
    }
}

fn mk_or(xs: Vec<T>) -> T {
    let mut v: Vec<T> = Vec::new();
    for x in xs {
        match x {
            T::False => {}
            T::True => return T::True,
            T::Or(ys) => v.extend(ys),
            x => v.push(x),
        }
    }
    v.sort();
    v.dedup();
    match v.len() {
        0 => T::False,
        1 => v.remove(0),
        _ => T::Or(v),
    }
}

fn mk_next(n: u32, x: T) -> T {
    if n == 0 || matches!(x, T::True | T::False) {
        x
    } else {
        T::Next(n, Box::new(x))
    }
}

/// The sequences of one property: the position automaton they share.
struct Seqs {
    nfa: Nfa,
    frags: Vec<Frag>,
}

impl Seqs {
    /// Where a sequence goes on a tick whose booleans hold as `v` says.
    fn step(&self, id: usize, state: &Option<Vec<usize>>, v: &dyn Fn(usize) -> bool) -> T {
        let f = &self.frags[id];
        let mut next: Vec<usize> = match state {
            None => f.first.clone(),
            Some(s) => {
                let mut r: Vec<usize> = Vec::new();
                for e in f.follow.iter().filter(|e| s.contains(&e.0)) {
                    if !r.contains(&e.1) {
                        r.push(e.1);
                    }
                }
                r
            }
        };
        next.retain(|&p| self.nfa.conds[p].iter().all(|&(k, pol)| v(k) == pol));
        if next.iter().any(|p| f.last.contains(p)) {
            T::True
        } else if next.is_empty() {
            T::False
        } else {
            next.sort_unstable();
            T::Seq(id, Some(next))
        }
    }

    fn leaves(&self, id: usize) -> BTreeSet<usize> {
        let f = &self.frags[id];
        f.pos
            .iter()
            .flat_map(|&p| self.nfa.conds[p].iter().map(|l| l.0))
            .collect()
    }
}

fn step(t: &T, v: &dyn Fn(usize) -> bool, sq: &Seqs) -> T {
    match t {
        T::True => T::True,
        T::False => T::False,
        T::Lit(k, pol) => {
            if v(*k) == *pol {
                T::True
            } else {
                T::False
            }
        }
        T::And(xs) => mk_and(xs.iter().map(|x| step(x, v, sq)).collect()),
        T::Or(xs) => mk_or(xs.iter().map(|x| step(x, v, sq)).collect()),
        T::Next(n, x) => mk_next(n - 1, (**x).clone()),
        T::Always(x) => mk_and(vec![step(x, v, sq), t.clone()]),
        T::Until(p, b, overlap) => {
            if v(*b) {
                if *overlap { step(p, v, sq) } else { T::True }
            } else {
                mk_and(vec![step(p, v, sq), t.clone()])
            }
        }
        T::Before(a, b, overlap) => {
            let (va, vb) = (v(*a), v(*b));
            if va && (!vb || *overlap) {
                T::True
            } else if vb {
                T::False
            } else {
                t.clone()
            }
        }
        T::Abort(p, b) => {
            if v(*b) {
                T::True
            } else {
                match step(p, v, sq) {
                    r @ (T::True | T::False) => r,
                    r => T::Abort(Box::new(r), *b),
                }
            }
        }
        T::NextEvent(c, n, p) => {
            if v(*c) {
                if *n == 1 {
                    step(p, v, sq)
                } else {
                    T::NextEvent(*c, n - 1, p.clone())
                }
            } else {
                t.clone()
            }
        }
        T::Seq(id, state) => sq.step(*id, state, v),
    }
}

fn term_leaves(t: &T, sq: &Seqs, out: &mut BTreeSet<usize>) {
    match t {
        T::True | T::False => {}
        T::Lit(k, _) => {
            out.insert(*k);
        }
        T::And(xs) | T::Or(xs) => xs.iter().for_each(|x| term_leaves(x, sq, out)),
        T::Next(_, x) | T::Always(x) => term_leaves(x, sq, out),
        T::Until(p, b, _) | T::Abort(p, b) => {
            out.insert(*b);
            term_leaves(p, sq, out);
        }
        T::Before(a, b, _) => {
            out.insert(*a);
            out.insert(*b);
        }
        T::NextEvent(c, _, p) => {
            out.insert(*c);
            term_leaves(p, sq, out);
        }
        T::Seq(id, _) => out.extend(sq.leaves(*id)),
    }
}

/// One tick of a term in a position where its conjuncts are separate obligations: the
/// failure of one is reported while the others go on. Returns what is left and whether
/// something failed now.
fn step_top(t: &T, v: &dyn Fn(usize) -> bool, sq: &Seqs) -> (T, bool) {
    match t {
        T::And(xs) => {
            let mut fail = false;
            let mut rest = Vec::new();
            for x in xs {
                let (r, f) = step_top(x, v, sq);
                fail |= f;
                rest.push(r);
            }
            (mk_and(rest), fail)
        }
        T::Always(x) => {
            let (r, f) = step_top(x, v, sq);
            (mk_and(vec![r, t.clone()]), f)
        }
        T::Or(xs) => {
            let c: Vec<T> = xs.iter().map(|x| step(x, v, sq)).collect();
            if c.iter().any(|r| *r != T::False) {
                return (mk_or(c), false);
            }
            // All alternatives failed. A lone alternative that is not just a failed boolean
            // (an implication whose left side held) goes on as separate obligations.
            let live: Vec<&T> = xs.iter().filter(|x| !matches!(x, T::Lit(..))).collect();
            if let [only] = live[..] {
                let (r, _) = step_top(only, v, sq);
                return (r, true);
            }
            (T::True, true)
        }
        _ => match step(t, v, sq) {
            T::False => (T::True, true),
            r => (r, false),
        },
    }
}

/// The conjuncts of a term.
fn conjuncts(t: T) -> Vec<T> {
    match t {
        T::True => Vec::new(),
        T::And(xs) => xs,
        x => vec![x],
    }
}

/// Where a live term goes on one valuation: whether a failure is reported, and the terms
/// that go on.
#[derive(Clone, PartialEq, Eq)]
struct Out {
    fail: bool,
    to: Vec<usize>,
}

fn to_term(pt: &Pt, sq: &mut Seqs) -> PResult<T> {
    Ok(match pt {
        Pt::False => T::False,
        Pt::Leaf(k) => T::Lit(*k, true),
        Pt::NotLeaf(k) => T::Lit(*k, false),
        Pt::Seq(s) => {
            let f = sq.nfa.trimmed(s)?;
            sq.frags.push(f);
            T::Seq(sq.frags.len() - 1, None)
        }
        Pt::And(xs) => mk_and(xs.iter().map(|x| to_term(x, sq)).collect::<PResult<_>>()?),
        Pt::Or(xs) => mk_or(xs.iter().map(|x| to_term(x, sq)).collect::<PResult<_>>()?),
        Pt::Imp(k, p) => mk_or(vec![T::Lit(*k, false), to_term(p, sq)?]),
        Pt::Iff(x, y) => mk_and(vec![
            mk_or(vec![T::Lit(*x, false), T::Lit(*y, true)]),
            mk_or(vec![T::Lit(*x, true), T::Lit(*y, false)]),
        ]),
        Pt::Next(n, p) => mk_next(*n, to_term(p, sq)?),
        Pt::NextA(lo, hi, p) => {
            let t = to_term(p, sq)?;
            mk_and((*lo..=*hi).map(|k| mk_next(k, t.clone())).collect())
        }
        Pt::NextE(lo, hi, p) => {
            let t = to_term(p, sq)?;
            mk_or((*lo..=*hi).map(|k| mk_next(k, t.clone())).collect())
        }
        Pt::NextEvent { c, lo, hi, all, p } => {
            let t = to_term(p, sq)?;
            let parts = (*lo..=*hi)
                .map(|k| T::NextEvent(*c, k, Box::new(t.clone())))
                .collect();
            if *all { mk_and(parts) } else { mk_or(parts) }
        }
        Pt::Until(p, k, ov) => T::Until(Box::new(to_term(p, sq)?), *k, *ov),
        Pt::Before(x, y, ov) => T::Before(*x, *y, *ov),
        Pt::SyncAbort(p, k) => match to_term(p, sq)? {
            r @ (T::True | T::False) => r,
            r => T::Abort(Box::new(r), *k),
        },
        Pt::Always(p) => T::Always(Box::new(to_term(p, sq)?)),
        Pt::Suffix { .. } => return Err(Recover),
    })
}

impl TProp {
    /// Does the directive start only at the first tick?
    pub(super) fn needs_first(&self) -> bool {
        !matches!(self.pt, Pt::Always(_))
            && !matches!(&self.pt, Pt::Suffix { rhs, .. } if matches!(**rhs, Pt::False))
    }
}

/// The checker process of a temporal directive.
pub(super) fn temporal_checker(
    prop: TProp,
    clock: Expr,
    report: Option<Expr>,
    severity: Option<Expr>,
    mut bi: Gen,
) -> PResult<ProcessStmt> {
    let TProp { pt, leaves } = prop;
    // The shape: [always] p, or [always] {l} |-> p.
    let (always, pt) = match pt {
        Pt::Always(p) => (true, *p),
        p => (false, p),
    };
    let (trig, overlap, body) = match pt {
        Pt::Suffix { lhs, overlap, rhs } => (Some(lhs), overlap, *rhs),
        p => (None, true, p),
    };
    // `never {s}` looks for matches from every tick.
    let always = always || matches!(body, Pt::False);
    let mut sq = Seqs {
        nfa: Nfa::default(),
        frags: Vec::new(),
    };
    let trig_frag = match &trig {
        Some(s) => Some(sq.nfa.trimmed(s)?),
        None => None,
    };
    let mut p0 = to_term(&body, &mut sq)?;
    if !overlap {
        p0 = mk_next(1, p0);
    }
    // `never {s}`: a failure at every match, no obligation to follow.
    let never_seq = p0 == T::False && trig_frag.is_some() && overlap;
    if p0 == T::False && !never_seq {
        return Err(Recover);
    }
    let start: Vec<T> = if never_seq { Vec::new() } else { conjuncts(p0) };
    // Every term reachable from the start, with where it goes on each valuation of the
    // booleans it tests.
    let mut atoms: Vec<T> = Vec::new();
    let mut index: HashMap<T, usize> = HashMap::new();
    let intern = |t: T, atoms: &mut Vec<T>, index: &mut HashMap<T, usize>| -> PResult<usize> {
        if let Some(&i) = index.get(&t) {
            return Ok(i);
        }
        if atoms.len() >= MAX_ATOMS {
            return Err(Recover);
        }
        atoms.push(t.clone());
        index.insert(t, atoms.len() - 1);
        Ok(atoms.len() - 1)
    };
    let mut start_idx = Vec::new();
    for t in start {
        start_idx.push(intern(t, &mut atoms, &mut index)?);
    }
    let mut table: Vec<(Vec<usize>, Vec<Out>)> = Vec::new();
    let mut steps = 0usize;
    let mut i = 0;
    while i < atoms.len() {
        let a = atoms[i].clone();
        let mut ls = BTreeSet::new();
        term_leaves(&a, &sq, &mut ls);
        let ls: Vec<usize> = ls.into_iter().collect();
        steps += 1 << ls.len();
        if ls.len() > MAX_TERM_LEAVES || steps > MAX_STEPS {
            return Err(Recover);
        }
        let mut outs = Vec::new();
        for m in 0..1usize << ls.len() {
            let v = |k: usize| {
                ls.iter()
                    .position(|&x| x == k)
                    .is_some_and(|b| m >> b & 1 == 1)
            };
            let (r, fail) = step_top(&a, &v, &sq);
            let mut to = Vec::new();
            for c in conjuncts(r) {
                let id = intern(c, &mut atoms, &mut index)?;
                if !to.contains(&id) {
                    to.push(id);
                }
            }
            to.sort_unstable();
            outs.push(Out { fail, to });
        }
        table.push((ls, outs));
        i += 1;
    }
    // The process.
    let g = &mut bi;
    let lv: Vec<String> = (0..leaves.exprs.len()).map(|k| vname("b", k)).collect();
    bools(g, &lv);
    let d = |a: usize| vname("d", a);
    let e = |a: usize| vname("e", a);
    let names: Vec<String> = (0..atoms.len()).flat_map(|a| [d(a), e(a)]).collect();
    bools(g, &names);
    bools(g, &["\\psl fail\\".to_string()]);
    let wait = g.stmt(SeqStmtKind::Wait {
        on: None,
        until: Some(clock),
        timeout: None,
    });
    let mut evals = Vec::new();
    for (k, x) in leaves.exprs.into_iter().enumerate() {
        evals.push(g.assign(&lv[k], g.var("false")));
        evals.push(g.when(x, vec![g.assign(&lv[k], g.var("true"))]));
    }
    let mut stmts: Vec<SeqStmt> = Vec::new();
    let start_gate = move |g: &Gen| {
        if always {
            g.var("true")
        } else {
            g.var("\\psl first\\")
        }
    };
    if let Some(f) = &trig_frag {
        bools(g, &["\\psl match\\".to_string()]);
        trigger(g, &sq.nfa, f, 1, start_gate, "\\psl match\\", &mut stmts);
    }
    let inject = |g: &Gen| {
        if trig_frag.is_some() {
            g.var("\\psl match\\")
        } else {
            start_gate(g)
        }
    };
    stmts.push(g.assign("\\psl fail\\", g.var("false")));
    if never_seq {
        stmts.push(g.assign("\\psl fail\\", inject(g)));
    }
    for a in 0..atoms.len() {
        stmts.push(g.assign(&e(a), g.var("false")));
    }
    for (a, (ls, outs)) in table.iter().enumerate() {
        let mut targets: Vec<&Out> = Vec::new();
        for o in outs {
            if (o.fail || !o.to.is_empty()) && !targets.contains(&o) {
                targets.push(o);
            }
        }
        let mut then = Vec::new();
        for t in targets {
            let sel: Vec<usize> = (0..outs.len()).filter(|&m| outs[m] == *t).collect();
            let cond = if sel.len() == outs.len() {
                g.var("true")
            } else {
                let terms = sel
                    .iter()
                    .map(|&m| {
                        let lits: Vec<(usize, bool)> = ls
                            .iter()
                            .enumerate()
                            .map(|(bit, &k)| (k, m >> bit & 1 == 1))
                            .collect();
                        cond_expr(g, &lits)
                    })
                    .collect();
                any_of(g, terms)
            };
            let mut set: Vec<SeqStmt> = Vec::new();
            if t.fail {
                set.push(g.assign("\\psl fail\\", g.var("true")));
            }
            set.extend(t.to.iter().map(|&x| g.assign(&e(x), g.var("true"))));
            then.push(g.when(cond, set));
        }
        if then.is_empty() {
            continue;
        }
        let guard = if start_idx.contains(&a) {
            let live = g.var(&d(a));
            any_of(g, vec![live, inject(g)])
        } else {
            g.var(&d(a))
        };
        stmts.push(g.when(guard, then));
    }
    for a in 0..atoms.len() {
        stmts.push(g.assign(&d(a), g.var(&e(a))));
    }
    let hit = g.var("\\psl fail\\");
    stmts.push(fail_stmt(g, hit, report, severity));
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
