//! Aggregates and choices (sem.md 3.4, LRM 9.3.3). Slice S3.
//!
//! - [`resolve_aggregate`]: a record or array aggregate whose type the context
//!   gave; the caller (the resolver in `resolve.rs`) passes itself as an
//!   [`AggResolver`] for the element values and choices.
//! - [`resolve_choices`], [`check_case`]: case statement and case generate
//!   choices (LRM 10.9, 11.8). Aggregate and case choices share one checker:
//!   choice values are folded to sorted `(lo, hi)` position intervals.
//! - [`recheck_case`]: the same checks with an elaboration-time evaluator for
//!   locally static choices sem cannot fold itself (Q3, the `ConstEval` hook).
//! - Folded array aggregates are compact (plan rule 12): run-length encoded, so
//!   `(others => (others => '0'))` of a 64K-entry memory is one run.
//!
//! HIR shape of n-dimensional aggregates: the value of an association of an
//! aggregate for dimension `k < n - 1` is itself an `ExprKind::Aggregate` (or a
//! string literal `Const` for the last dimension) whose `subtype` is the
//! outermost aggregate's context subtype; its dimension is its nesting depth.

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast;

use crate::db::{Db, Reported};
use crate::decl::Designator;
use crate::diag;
use crate::expr::{CandSet, ClassMask, Expected};
use crate::hir::{AggAssoc, AggBounds, AttrPrefix, Choice, ChoiceSet, Expr, ExprKind};
use crate::ids::{ExprId, SubtypeId, TypeId};
use crate::stat::{ArrayConst, ConstValue, Elems, Staticness};
use crate::types::{Constraint, Direction, RangeC, SubtypeDef, TypeKind};

// ---------------------------------------------------------------------------
// Static ranges

/// A folded discrete range, in positions (enumeration literals by position).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticRange {
    pub left: i64,
    pub dir: Direction,
    pub right: i64,
}

impl StaticRange {
    pub fn new(left: i64, dir: Direction, right: i64) -> Self {
        StaticRange { left, dir, right }
    }

    pub fn low(&self) -> i64 {
        match self.dir {
            Direction::To => self.left,
            Direction::Downto => self.right,
        }
    }

    pub fn high(&self) -> i64 {
        match self.dir {
            Direction::To => self.right,
            Direction::Downto => self.left,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.is_null()
    }

    pub fn is_null(&self) -> bool {
        self.low() > self.high()
    }

    /// Number of values (an i64 range can hold 2^64 of them).
    pub fn len(&self) -> u128 {
        if self.is_null() {
            0
        } else {
            (i128::from(self.high()) - i128::from(self.low()) + 1) as u128
        }
    }

    pub fn contains(&self, v: i64) -> bool {
        self.low() <= v && v <= self.high()
    }

    fn reversed(self) -> Self {
        let dir = match self.dir {
            Direction::To => Direction::Downto,
            Direction::Downto => Direction::To,
        };
        StaticRange::new(self.right, dir, self.left)
    }

    /// Offset of index value `v` from the left end.
    pub(crate) fn offset(&self, v: i64) -> u64 {
        match self.dir {
            Direction::To => (i128::from(v) - i128::from(self.left)) as u64,
            Direction::Downto => (i128::from(self.left) - i128::from(v)) as u64,
        }
    }
}

/// The folded value of a constraint range, when both bounds are folded.
pub fn range_value(db: &Db, r: &RangeC) -> Option<StaticRange> {
    match r {
        RangeC::Explicit { left, dir, right } => Some(StaticRange::new(
            db.value(*left)?.as_int()?,
            *dir,
            db.value(*right)?.as_int()?,
        )),
        RangeC::Attr {
            prefix,
            reverse,
            dim,
        } => {
            let st = match *prefix {
                AttrPrefix::Subtype(st) => st,
                AttrPrefix::Expr(e) => db.exprs[e].subtype,
                AttrPrefix::Entity(_) => return None,
            };
            // `dim` is 1-based (0 for a subtype choice of a scalar type).
            let r = subtype_range(db, st, dim.saturating_sub(1) as usize)?;
            Some(if *reverse { r.reversed() } else { r })
        }
    }
}

/// The folded range of dimension `dim` of an array subtype, or (`dim == 0`) of
/// a scalar subtype.
pub fn subtype_range(db: &Db, st: SubtypeId, dim: usize) -> Option<StaticRange> {
    let def = &db.subtypes[st];
    match &def.constraint {
        Constraint::Range(r) if dim == 0 => range_value(db, r),
        Constraint::Array { dims: Some(d), .. } => range_value(db, d.get(dim)?),
        Constraint::None if dim == 0 => {
            let ty = &db.types[def.base];
            match &ty.kind {
                TypeKind::Enum { lits } => {
                    Some(StaticRange::new(0, Direction::To, lits.len() as i64 - 1))
                }
                k if k.is_scalar() && ty.base_subtype != st => {
                    subtype_range(db, ty.base_subtype, 0)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The range of the base type of a discrete type: every literal of an
/// enumeration type; for an anonymous integer base type the smallest of 32 or
/// 64 bits that holds its first subtype (sem.md Q2).
pub fn type_range(db: &Db, ty: TypeId) -> Option<StaticRange> {
    let def = &db.types[ty];
    match &def.kind {
        TypeKind::Enum { lits } => Some(StaticRange::new(0, Direction::To, lits.len() as i64 - 1)),
        TypeKind::UniversalInteger => Some(StaticRange::new(i64::MIN, Direction::To, i64::MAX)),
        TypeKind::Integer => {
            if def.base_subtype != def.first_subtype
                && let Some(r) = subtype_range(db, def.base_subtype, 0)
            {
                return Some(r);
            }
            let first = subtype_range(db, def.first_subtype, 0)?;
            if !def.anonymous {
                return Some(first);
            }
            let fits32 = first.is_null()
                || (first.low() >= i64::from(i32::MIN) && first.high() <= i64::from(i32::MAX));
            Some(if fits32 {
                StaticRange::new(i64::from(i32::MIN), Direction::To, i64::from(i32::MAX))
            } else {
                StaticRange::new(i64::MIN, Direction::To, i64::MAX)
            })
        }
        _ => None,
    }
}

/// The values a case statement must cover (10.9): the subtype of the
/// expression when it is static, otherwise its base type.
pub fn case_domain(db: &Db, st: SubtypeId) -> Option<StaticRange> {
    subtype_range(db, st, 0).or_else(|| type_range(db, db.base(st)))
}

// ---------------------------------------------------------------------------
// Compact array constants (rule 12)

/// Longest 1-D enumeration array kept as [`Elems::Bytes`]; longer ones are runs.
const BYTES_MAX: u64 = 1 << 16;
/// Folding gives up (the value stays a tree for M2) beyond this many runs.
const RUNS_MAX: usize = 1 << 20;

/// Build an array constant in canonical form: 1-D arrays of an enumeration
/// type with at most 256 literals and at most 64K elements as bytes, all
/// others as merged runs. Every array constant sem builds goes through here, so
/// equal values compare equal.
pub fn array_const(
    db: &Db,
    elem_ty: TypeId,
    bounds: Vec<StaticRange>,
    runs: Vec<(u64, ConstValue)>,
) -> ArrayConst {
    let runs = merge_runs(runs);
    let len: u64 = runs.iter().map(|r| r.0).sum();
    let small_enum =
        matches!(&db.types[elem_ty].kind, TypeKind::Enum { lits } if lits.len() <= 256);
    let elems = if bounds.len() == 1 && small_enum && len <= BYTES_MAX {
        let mut bytes = Vec::with_capacity(len as usize);
        for (n, v) in &runs {
            let b = match v {
                ConstValue::Enum(p) => *p as u8,
                _ => unreachable!("enumeration array element is ConstValue::Enum"),
            };
            bytes.extend(std::iter::repeat_n(b, *n as usize));
        }
        Elems::Bytes(bytes.into())
    } else {
        Elems::Runs(runs.into())
    };
    ArrayConst {
        bounds: bounds.iter().map(|r| (r.left, r.dir, r.right)).collect(),
        elems,
    }
}

/// The elements of an array constant as runs, row-major.
pub fn elem_runs(a: &ArrayConst) -> Vec<(u64, ConstValue)> {
    match &a.elems {
        Elems::Bytes(b) => merge_runs(
            b.iter()
                .map(|&x| (1, ConstValue::Enum(u32::from(x))))
                .collect(),
        ),
        Elems::Runs(r) => r.to_vec(),
    }
}

/// Element positions of a 1-D array constant of a discrete element type, up to
/// `max` elements.
pub fn array_positions(a: &ArrayConst, max: u64) -> Option<Vec<i64>> {
    let runs = elem_runs(a);
    if runs.iter().map(|r| r.0).sum::<u64>() > max {
        return None;
    }
    let mut out = Vec::new();
    for (n, v) in runs {
        let p = v.as_int()?;
        out.extend(std::iter::repeat_n(p, n as usize));
    }
    Some(out)
}

fn merge_runs(runs: Vec<(u64, ConstValue)>) -> Vec<(u64, ConstValue)> {
    let mut out: Vec<(u64, ConstValue)> = Vec::with_capacity(runs.len());
    for (n, v) in runs {
        if n == 0 {
            continue;
        }
        match out.last_mut() {
            Some((m, w)) if *w == v => *m += n,
            _ => out.push((n, v)),
        }
    }
    out
}

fn array_len(bounds: &[(i64, Direction, i64)]) -> u64 {
    bounds
        .iter()
        .map(|&(l, d, r)| StaticRange::new(l, d, r).len() as u64)
        .product()
}

// ---------------------------------------------------------------------------
// The resolver interface

/// What aggregate analysis needs from expression resolution (`resolve.rs`).
pub trait AggResolver {
    /// Bottom-up candidate types of an element value (3.1).
    fn cands(&mut self, db: &mut Db, e: &ast::Expr) -> CandSet;
    /// Top-down resolution of an element value or choice (3.2).
    fn resolve(&mut self, db: &mut Db, e: &ast::Expr, exp: Expected) -> Result<ExprId, Reported>;
    /// A discrete range choice, its bounds resolved as `index_ty`.
    fn discrete_range(
        &mut self,
        db: &mut Db,
        r: &ast::DiscreteRange,
        index_ty: TypeId,
    ) -> Result<RangeC, Reported>;
    /// `Some` when the choice expression is a type mark (a subtype choice).
    fn type_mark(&mut self, db: &mut Db, e: &ast::Expr) -> Option<SubtypeId>;
}

/// The elaboration-time evaluator for locally static expressions sem cannot
/// fold (the 2008 ieee calls, Q3). M2's interpreter implements it.
pub trait ConstEval {
    fn eval(&mut self, db: &Db, e: ExprId) -> Option<ConstValue>;
}

/// Sem's own evaluator: the folded value cache.
struct Folded;

impl ConstEval for Folded {
    fn eval(&mut self, db: &Db, e: ExprId) -> Option<ConstValue> {
        db.value(e).cloned()
    }
}

/// `E1320`: an aggregate where the context gives no type (`Expected::Any`).
pub fn no_context(db: &mut Db, span: Span) -> Reported {
    db.error(
        Diagnostic::error(
            diag::AGG_NO_TYPE,
            "the type of this aggregate cannot be determined",
        )
        .with_primary(span, "the context does not give a type")
        .with_help("qualify it with its type: `t'(...)`"),
    )
}

// ---------------------------------------------------------------------------
// Small helpers

fn mk_expr(
    db: &mut Db,
    kind: ExprKind,
    subtype: SubtypeId,
    span: Span,
    stat: Staticness,
    value: Option<ConstValue>,
) -> ExprId {
    let value = value.map(|v| db.add_const(v));
    db.add_expr(Expr {
        kind,
        subtype,
        span,
        stat,
        value,
    })
}

fn mk_const(db: &mut Db, v: ConstValue, st: SubtypeId, span: Span) -> ExprId {
    let c = db.add_const(v);
    db.add_expr(Expr {
        kind: ExprKind::Const(c),
        subtype: st,
        span,
        stat: Staticness::Locally,
        value: Some(c),
    })
}

pub(crate) fn type_name(db: &Db, ty: TypeId) -> String {
    let t = &db.types[ty];
    if t.anonymous {
        format!("the base type of `{}`", t.name.text)
    } else {
        format!("`{}`", t.name.text)
    }
}

/// A position of type `ty` as the user writes it: a literal name or a number.
fn show_pos(db: &Db, ty: TypeId, v: i64) -> String {
    if let TypeKind::Enum { lits } = &db.types[ty].kind
        && let Some(&d) = usize::try_from(v).ok().and_then(|i| lits.get(i))
    {
        return db.decls[d].name.to_string();
    }
    v.to_string()
}

fn show_interval(db: &Db, ty: TypeId, lo: i64, hi: i64) -> String {
    if lo == hi {
        show_pos(db, ty, lo)
    } else {
        format!("{} to {}", show_pos(db, ty, lo), show_pos(db, ty, hi))
    }
}

fn is_error(db: &Db, ty: TypeId) -> bool {
    matches!(db.types[ty].kind, TypeKind::Error)
}

/// A 1-D array whose element type is an enumeration type with character
/// literals (strings, bit and logic vectors).
fn is_string_like(db: &Db, ty: TypeId) -> bool {
    match &db.types[ty].kind {
        TypeKind::Array { indexes, elem } if indexes.len() == 1 => {
            match &db.types[db.base(*elem)].kind {
                TypeKind::Enum { lits } => lits
                    .iter()
                    .any(|&l| matches!(db.decls[l].name, Designator::Char(_))),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Classes type `ty` belongs to, for context-typed literal forms.
fn class_of(db: &Db, ty: TypeId) -> ClassMask {
    let m = match &db.types[ty].kind {
        TypeKind::UniversalInteger | TypeKind::Integer => ClassMask::ANY_INTEGER.0,
        TypeKind::UniversalReal | TypeKind::Floating => ClassMask::ANY_REAL.0,
        TypeKind::Access { .. } => ClassMask::ANY_ACCESS.0,
        TypeKind::Array { .. } => {
            let s = if is_string_like(db, ty) {
                ClassMask::STRING_LIKE.0
            } else {
                0
            };
            ClassMask::ANY_COMPOSITE.0 | ClassMask::ANY_ARRAY.0 | s
        }
        TypeKind::Record { .. } => ClassMask::ANY_COMPOSITE.0 | ClassMask::ANY_RECORD.0,
        _ => 0,
    };
    ClassMask(m)
}

/// Does candidate set `c` admit type `ty` (sem.md 3.1 `compatible`)?
fn fits(db: &Db, c: &CandSet, ty: TypeId) -> bool {
    if is_error(db, ty) {
        return true;
    }
    let target = &db.types[ty].kind;
    match c {
        CandSet::Types(v) => v.iter().any(|i| {
            i.ty == ty
                || match db.types[i.ty].kind {
                    TypeKind::Error => true,
                    TypeKind::UniversalInteger => matches!(target, TypeKind::Integer),
                    TypeKind::UniversalReal => matches!(target, TypeKind::Floating),
                    _ => false,
                }
        }),
        CandSet::Class(m) => m.0 & class_of(db, ty).0 != 0,
        CandSet::Alloc(d) => db.alloc_admits(*d, ty),
    }
}

fn is_empty(c: &CandSet) -> bool {
    matches!(c, CandSet::Types(v) if v.is_empty())
}

/// The span of a resolved choice, for diagnostics.
fn choice_span(db: &Db, c: &Choice, fallback: Span) -> Span {
    match c {
        Choice::Expr(e) => db.exprs[*e].span,
        Choice::Range(RangeC::Explicit { left, right, .. }) => {
            db.exprs[*left].span.to(db.exprs[*right].span)
        }
        Choice::Range(RangeC::Attr { prefix, .. }) => match *prefix {
            AttrPrefix::Expr(e) => db.exprs[e].span,
            AttrPrefix::Subtype(st) if db.subtypes[st].span != Span::default() => {
                db.subtypes[st].span
            }
            _ => fallback,
        },
        Choice::Element(_) | Choice::Others => fallback,
    }
}

pub(crate) fn range_stat(db: &Db, r: &RangeC) -> Staticness {
    match r {
        RangeC::Explicit { left, right, .. } => db.exprs[*left].stat.min(db.exprs[*right].stat),
        RangeC::Attr { .. } => {
            if range_value(db, r).is_some() {
                Staticness::Locally
            } else {
                Staticness::Globally
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Choices

/// Resolve the choices of one case alternative or aggregate association
/// against `ty`: the index type of an array aggregate, or the type of a case
/// expression. A type mark choice becomes the range of that subtype.
pub fn resolve_choices(
    db: &mut Db,
    r: &mut dyn AggResolver,
    choices: &[ast::Choice],
    ty: TypeId,
) -> Result<Vec<Choice>, Reported> {
    let mut out = Vec::with_capacity(choices.len());
    let mut failed = None;
    for c in choices {
        let res = match c {
            ast::Choice::Others(_) => Ok(Choice::Others),
            ast::Choice::Range(dr) => r.discrete_range(db, dr, ty).map(Choice::Range),
            ast::Choice::Expr(e) => match r.type_mark(db, e) {
                Some(st) => {
                    let base = db.base(st);
                    if base != ty && !is_error(db, base) && !is_error(db, ty) {
                        Err(db.error(
                            Diagnostic::error(diag::TYPE_MISMATCH, "choice has the wrong type")
                                .with_primary(
                                    e.span,
                                    format!(
                                        "expected a subtype of {}, found a subtype of {}",
                                        type_name(db, ty),
                                        type_name(db, base)
                                    ),
                                ),
                        ))
                    } else {
                        Ok(Choice::Range(RangeC::Attr {
                            prefix: AttrPrefix::Subtype(st),
                            reverse: false,
                            dim: 1,
                        }))
                    }
                }
                None => r.resolve(db, e, Expected::Type(ty)).map(Choice::Expr),
            },
        };
        match res {
            Ok(c) => out.push(c),
            Err(e) => failed = Some(e),
        }
    }
    match failed {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctx {
    Aggregate,
    Case,
}

impl Ctx {
    fn overlap(self) -> &'static str {
        match self {
            Ctx::Aggregate => diag::CHOICE_OVERLAP,
            Ctx::Case => diag::CASE_COVERAGE,
        }
    }
    fn uncovered(self) -> &'static str {
        match self {
            Ctx::Aggregate => diag::CHOICE_UNCOVERED,
            Ctx::Case => diag::CASE_COVERAGE,
        }
    }
    fn outside(self) -> &'static str {
        match self {
            Ctx::Aggregate => diag::OUT_OF_SUBTYPE,
            Ctx::Case => diag::CASE_COVERAGE,
        }
    }
}

/// What the choices must cover.
#[derive(Clone, Copy, Debug)]
enum Coverage {
    /// Every value of the range (case statements without `others`).
    Full(StaticRange),
    /// Every value between the smallest and largest choice (named array
    /// aggregates without `others`).
    Contiguous,
    /// Nothing (an `others` choice is present).
    None,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    lo: i64,
    hi: i64,
    alt: usize,
    span: Span,
}

/// Fold the choices of every alternative to intervals.
/// `Ok(None)`: some choice is locally static but not foldable here (Q3).
/// `strict`: a choice that is not locally static is an error (`E1410`).
fn fold_items(
    db: &mut Db,
    eval: &mut dyn ConstEval,
    alts: &[&[Choice]],
    strict: bool,
    span: Span,
) -> Result<Option<Vec<Item>>, Reported> {
    let mut items = Vec::new();
    let mut deferred = false;
    let mut failed = None;
    for (alt, choices) in alts.iter().enumerate() {
        for c in choices.iter() {
            let sp = choice_span(db, c, span);
            let (val, stat) = match c {
                Choice::Expr(e) => {
                    let v = eval.eval(db, *e).and_then(|v| v.as_int()).map(|v| (v, v));
                    (v, db.exprs[*e].stat)
                }
                Choice::Range(r) => {
                    let v = match r {
                        RangeC::Explicit { left, dir, right } => {
                            match (eval.eval(db, *left), eval.eval(db, *right)) {
                                (Some(l), Some(rt)) => match (l.as_int(), rt.as_int()) {
                                    (Some(l), Some(rt)) => {
                                        let s = StaticRange::new(l, *dir, rt);
                                        Some((s.low(), s.high()))
                                    }
                                    _ => None,
                                },
                                _ => None,
                            }
                        }
                        RangeC::Attr { .. } => range_value(db, r).map(|s| (s.low(), s.high())),
                    };
                    (v, range_stat(db, r))
                }
                Choice::Others | Choice::Element(_) => continue,
            };
            match val {
                Some((lo, hi)) => items.push(Item {
                    lo,
                    hi,
                    alt,
                    span: sp,
                }),
                None if stat == Staticness::Locally => deferred = true,
                None if strict => {
                    failed = Some(
                        db.error(
                            Diagnostic::error(
                                diag::NOT_STATIC,
                                "case choice is not locally static",
                            )
                            .with_primary(sp, "not locally static")
                            .with_note("case choices must be locally static (LRM 10.9)"),
                        ),
                    );
                }
                None => deferred = true,
            }
        }
    }
    match failed {
        Some(e) => Err(e),
        None => Ok((!deferred).then_some(items)),
    }
}

/// Sorted, merged position intervals of each alternative.
type Intervals = Vec<Vec<(i64, i64)>>;
/// Element runs, row-major.
type Runs = Vec<(u64, ConstValue)>;

/// Overlap, range and coverage checks on folded choices. Returns the sorted,
/// merged intervals of each alternative.
#[allow(clippy::too_many_arguments)]
fn check_items(
    db: &mut Db,
    mut items: Vec<Item>,
    n_alts: usize,
    ty: TypeId,
    cov: Coverage,
    within: Option<StaticRange>,
    ctx: Ctx,
    span: Span,
) -> Result<Vec<Vec<(i64, i64)>>, Reported> {
    let mut failed = None;
    items.retain(|i| i.lo <= i.hi);
    if let Some(w) = within {
        for i in &items {
            if !w.contains(i.lo) || !w.contains(i.hi) {
                let what = match ctx {
                    Ctx::Aggregate => "index",
                    Ctx::Case => "case choice",
                };
                failed = Some(
                    db.error(
                        Diagnostic::error(
                            ctx.outside(),
                            format!(
                                "{what} {} is outside {}",
                                show_interval(db, ty, i.lo, i.hi),
                                show_interval(db, ty, w.low(), w.high())
                            ),
                        )
                        .with_primary(i.span, "out of range"),
                    ),
                );
            }
        }
    }
    items.sort_by_key(|i| (i.lo, i.hi));
    // Overlaps: compare each choice with the furthest-reaching earlier one.
    let mut reach: Option<(i64, Span)> = None;
    for i in &items {
        if let Some((hi, sp)) = reach {
            if i.lo <= hi {
                let dup_hi = i.hi.min(hi);
                failed = Some(
                    db.error(
                        Diagnostic::error(
                            ctx.overlap(),
                            format!("{} is already covered", show_interval(db, ty, i.lo, dup_hi)),
                        )
                        .with_primary(i.span, "covered again here")
                        .with_secondary(sp, "first covered here"),
                    ),
                );
            }
            if i.hi > hi {
                reach = Some((i.hi, i.span));
            }
        } else {
            reach = Some((i.hi, i.span));
        }
    }
    let (lo, hi) = match cov {
        Coverage::Full(r) if !r.is_null() => (Some(r.low()), r.high()),
        Coverage::Contiguous if !items.is_empty() => (
            Some(items[0].lo),
            items.iter().map(|i| i.hi).max().expect("non-empty"),
        ),
        _ => (None, 0),
    };
    if let Some(mut next) = lo {
        let mut gaps = Vec::new();
        let mut done = false;
        for i in &items {
            if i.lo > next && !done {
                gaps.push((next, (i.lo - 1).min(hi)));
            }
            if i.hi >= hi {
                done = true;
            }
            if i.hi >= next {
                match i.hi.checked_add(1) {
                    Some(n) => next = n,
                    None => done = true,
                }
            }
        }
        if !done && next <= hi {
            gaps.push((next, hi));
        }
        gaps.retain(|g| g.0 <= g.1);
        if !gaps.is_empty() {
            let shown: Vec<String> = gaps
                .iter()
                .take(4)
                .map(|&(a, b)| show_interval(db, ty, a, b))
                .collect();
            let more = if gaps.len() > 4 {
                format!(" and {} more", gaps.len() - 4)
            } else {
                String::new()
            };
            let mut d = Diagnostic::error(
                ctx.uncovered(),
                format!("missing choices for {}{more}", shown.join(", ")),
            )
            .with_primary(span, "not every value is covered");
            d = match (ctx, cov) {
                (Ctx::Aggregate, _) => d.with_note(
                    "the choices of a named array aggregate must form a contiguous range (LRM 9.3.3.3)",
                ),
                (Ctx::Case, _) => d.with_help("add the missing values or `when others =>`"),
            };
            failed = Some(db.error(d));
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    let mut per_alt = vec![Vec::new(); n_alts];
    for i in items {
        let v: &mut Vec<(i64, i64)> = &mut per_alt[i.alt];
        match v.last_mut() {
            Some(last) if last.1.checked_add(1) == Some(i.lo) => last.1 = i.hi,
            _ => v.push((i.lo, i.hi)),
        }
    }
    Ok(per_alt)
}

/// `others` must be the last alternative and its only choice.
fn check_others_last(
    db: &mut Db,
    alts: &[&[Choice]],
    ctx: Ctx,
    span: Span,
) -> Result<bool, Reported> {
    let mut has = false;
    for (i, choices) in alts.iter().enumerate() {
        if choices.iter().any(|c| matches!(c, Choice::Others)) {
            has = true;
            if i + 1 != alts.len() || choices.len() != 1 {
                let code = match ctx {
                    Ctx::Aggregate => diag::AGG_OTHERS,
                    Ctx::Case => diag::CASE_COVERAGE,
                };
                return Err(db.error(
                    Diagnostic::error(code, "`others` must be the last choice and stand alone")
                        .with_primary(span, "misplaced `others`"),
                ));
            }
        }
    }
    Ok(has)
}

/// Is `ty` `std_ulogic` (or a type with the same nine literals)?
fn is_std_ulogic(db: &Db, ty: TypeId) -> bool {
    const LITS: [char; 9] = ['U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-'];
    match &db.types[ty].kind {
        TypeKind::Enum { lits } => {
            lits.len() == 9
                && lits
                    .iter()
                    .zip(LITS)
                    .all(|(&d, c)| db.decls[d].name == Designator::Char(c))
        }
        _ => false,
    }
}

/// Position of `'-'` in `std_ulogic`.
const DONT_CARE: i64 = 8;

/// Analyze the choices of a case statement (10.9) or case generate (11.8):
/// `alts` holds the resolved choices of each alternative, in order. Locally
/// static choices sem cannot fold (Q3) leave `intervals` empty; elab then
/// calls [`recheck_case`].
pub fn check_case(
    db: &mut Db,
    expr_subtype: SubtypeId,
    matching: bool,
    alts: Vec<Box<[Choice]>>,
    span: Span,
) -> Result<Vec<ChoiceSet>, Reported> {
    let refs: Vec<&[Choice]> = alts.iter().map(|a| &**a).collect();
    let intervals = case_core(db, &mut Folded, expr_subtype, matching, &refs, span, true)?;
    Ok(alts
        .into_iter()
        .enumerate()
        .map(|(i, choices)| ChoiceSet {
            choices,
            intervals: intervals.as_ref().map(|v| v[i].clone().into()),
        })
        .collect())
}

/// The checks of [`check_case`] with an elaboration-time evaluator (Q3).
pub fn recheck_case(
    db: &mut Db,
    eval: &mut dyn ConstEval,
    expr_subtype: SubtypeId,
    matching: bool,
    alts: &[&ChoiceSet],
    span: Span,
) -> Result<(), Reported> {
    let refs: Vec<&[Choice]> = alts.iter().map(|a| &*a.choices).collect();
    case_core(db, eval, expr_subtype, matching, &refs, span, false).map(|_| ())
}

/// Returns per-alternative intervals for discrete cases when every choice
/// folded; `None` for array cases and deferred ones.
fn case_core(
    db: &mut Db,
    eval: &mut dyn ConstEval,
    st: SubtypeId,
    matching: bool,
    alts: &[&[Choice]],
    span: Span,
    defer: bool,
) -> Result<Option<Intervals>, Reported> {
    let ty = db.base(st);
    if is_error(db, ty) {
        return Ok(None);
    }
    let has_others = check_others_last(db, alts, Ctx::Case, span)?;
    let discrete = db.types[ty].kind.is_discrete();
    let arr_elem = match &db.types[ty].kind {
        TypeKind::Array { indexes, elem }
            if indexes.len() == 1 && db.types[db.base(*elem)].kind.is_discrete() =>
        {
            Some(*elem)
        }
        _ => None,
    };
    if discrete {
        if matching && db.well_known.bit != Some(ty) && !is_std_ulogic(db, ty) {
            return Err(bad_matching(db, ty, span));
        }
        let Some(items) = fold_items(db, eval, alts, true, span)? else {
            if defer {
                return Ok(None);
            }
            return Err(db.ice(span, "case choice still not folded at elaboration"));
        };
        let domain = case_domain(db, st);
        if matching && is_std_ulogic(db, ty) {
            return std_ulogic_matching(db, items, alts.len(), ty, has_others, span).map(Some);
        }
        let cov = match (has_others, domain) {
            (true, _) => Coverage::None,
            (false, Some(r)) => Coverage::Full(r),
            (false, None) => {
                return Err(db.error(
                    Diagnostic::error(diag::CASE_COVERAGE, "case needs `when others`")
                        .with_primary(span, "the range of this expression's type is not known")
                        .with_help("add `when others =>`"),
                ));
            }
        };
        let within = subtype_range(db, st, 0);
        check_items(db, items, alts.len(), ty, cov, within, Ctx::Case, span).map(Some)
    } else if let Some(elem) = arr_elem {
        if matching && db.well_known.bit != Some(db.base(elem)) && !is_std_ulogic(db, db.base(elem))
        {
            return Err(bad_matching(db, ty, span));
        }
        array_case(db, eval, st, elem, matching, alts, has_others, span, defer)?;
        Ok(None)
    } else {
        check_case_subject(db, st, span)?;
        Err(db.ice(
            span,
            "case subject passed its type check but is not discrete",
        ))
    }
}

/// The subject of a case is discrete or a one-dimensional array of a
/// discrete type (10.9); `span` is the subject.
pub fn check_case_subject(db: &mut Db, st: SubtypeId, span: Span) -> Result<(), Reported> {
    let ty = db.base(st);
    let ok = is_error(db, ty)
        || db.types[ty].kind.is_discrete()
        || matches!(&db.types[ty].kind, TypeKind::Array { indexes, elem }
            if indexes.len() == 1 && db.types[db.base(*elem)].kind.is_discrete());
    if ok {
        return Ok(());
    }
    Err(db.error(
        Diagnostic::error(
            diag::TYPE_MISMATCH,
            format!(
                "case expression must be discrete or a one-dimensional array of a discrete type, found {}",
                type_name(db, ty)
            ),
        )
        .with_primary(span, "not discrete"),
    ))
}

/// `others` is the last alternative of a case and its only choice (10.9);
/// reported at the misplaced `others`.
pub fn check_case_others(db: &mut Db, alts: &[&[ast::Choice]]) -> Result<(), Reported> {
    for (i, choices) in alts.iter().enumerate() {
        for c in choices.iter() {
            if let ast::Choice::Others(sp) = c
                && (i + 1 != alts.len() || choices.len() != 1)
            {
                return Err(db.error(
                    Diagnostic::error(
                        diag::CASE_COVERAGE,
                        "`others` must be the last alternative and stand alone",
                    )
                    .with_primary(*sp, "misplaced `others`"),
                ));
            }
        }
    }
    Ok(())
}

fn bad_matching(db: &mut Db, ty: TypeId, span: Span) -> Reported {
    db.error(
        Diagnostic::error(
            diag::CASE_COVERAGE,
            format!(
                "`case?` expression of type {} must be BIT, STD_ULOGIC or a one-dimensional array of them",
                type_name(db, ty)
            ),
        )
        .with_primary(span, "in this matching case statement"),
    )
}

/// `case?` on `std_ulogic` (10.9): a `'-'` choice matches every value;
/// otherwise choices are compared by value. As sim1, choices overlapping only
/// through `'-'` are accepted.
fn std_ulogic_matching(
    db: &mut Db,
    items: Vec<Item>,
    n_alts: usize,
    ty: TypeId,
    has_others: bool,
    span: Span,
) -> Result<Vec<Vec<(i64, i64)>>, Reported> {
    let wildcard = items.iter().any(|i| i.lo <= DONT_CARE && DONT_CARE <= i.hi);
    let exact: Vec<Item> = items
        .iter()
        .copied()
        .filter(|i| !(i.lo <= DONT_CARE && DONT_CARE <= i.hi))
        .collect();
    let cov = if has_others || wildcard {
        Coverage::None
    } else {
        Coverage::Full(StaticRange::new(0, Direction::To, DONT_CARE))
    };
    check_items(db, exact, n_alts, ty, cov, None, Ctx::Case, span)?;
    // Intervals keep every choice (the `'-'` ones too), sorted per alternative.
    let mut per_alt = vec![Vec::new(); n_alts];
    let mut items = items;
    items.sort_by_key(|i| (i.lo, i.hi));
    for i in items {
        per_alt[i.alt].push((i.lo, i.hi));
    }
    Ok(per_alt)
}

/// Case on a 1-D array (10.9): choices are locally static values of the same
/// length; duplicates are errors, and without `others` every value of the
/// element subtype at that length must appear.
#[allow(clippy::too_many_arguments)]
fn array_case(
    db: &mut Db,
    eval: &mut dyn ConstEval,
    st: SubtypeId,
    elem: SubtypeId,
    matching: bool,
    alts: &[&[Choice]],
    has_others: bool,
    span: Span,
    defer: bool,
) -> Result<(), Reported> {
    const MAX_LEN: u64 = 1 << 16;
    let elem_ty = db.base(elem);
    let expected_len = subtype_range(db, st, 0).map(|r| r.len());
    let wild = matching && is_std_ulogic(db, elem_ty);
    let mut seen: HashMap<Vec<i64>, Span> = HashMap::new();
    let mut len: Option<(u128, Span)> = None;
    let mut failed = None;
    let mut deferred = false;
    let mut patterns = Vec::new();
    for choices in alts {
        for c in choices.iter() {
            let sp = choice_span(db, c, span);
            let e = match c {
                Choice::Others => continue,
                Choice::Expr(e) => *e,
                Choice::Range(_) | Choice::Element(_) => {
                    failed = Some(
                        db.error(
                            Diagnostic::error(
                                diag::CASE_COVERAGE,
                                "a range is not a choice of a case on an array",
                            )
                            .with_primary(sp, "range choice"),
                        ),
                    );
                    continue;
                }
            };
            let pos = match eval.eval(db, e) {
                Some(ConstValue::Array(a)) => array_positions(&a, MAX_LEN),
                _ => None,
            };
            let Some(pos) = pos else {
                if db.exprs[e].stat == Staticness::Locally {
                    deferred = true;
                } else {
                    failed = Some(
                        db.error(
                            Diagnostic::error(
                                diag::NOT_STATIC,
                                "case choice is not locally static",
                            )
                            .with_primary(sp, "not locally static")
                            .with_note("case choices must be locally static (LRM 10.9)"),
                        ),
                    );
                }
                continue;
            };
            let n = pos.len() as u128;
            if let Some(want) = expected_len
                && want != n
            {
                failed = Some(
                    db.error(
                        Diagnostic::error(
                            diag::LENGTH_MISMATCH,
                            format!("case choice has {n} elements, the expression has {want}"),
                        )
                        .with_primary(sp, "wrong length"),
                    ),
                );
                continue;
            }
            match len {
                Some((m, first)) if m != n => {
                    failed = Some(
                        db.error(
                            Diagnostic::error(
                                diag::LENGTH_MISMATCH,
                                format!("case choice has {n} elements, an earlier one has {m}"),
                            )
                            .with_primary(sp, "different length")
                            .with_secondary(first, "this one"),
                        ),
                    );
                    continue;
                }
                None => len = Some((n, sp)),
                _ => {}
            }
            if let Some(&first) = seen.get(&pos) {
                failed = Some(
                    db.error(
                        Diagnostic::error(diag::CASE_COVERAGE, "duplicate case choice")
                            .with_primary(sp, "covered again here")
                            .with_secondary(first, "first covered here"),
                    ),
                );
                continue;
            }
            seen.insert(pos.clone(), sp);
            patterns.push(pos);
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    if deferred {
        return if defer {
            Ok(())
        } else {
            Err(db.ice(span, "case choice still not folded at elaboration"))
        };
    }
    if has_others {
        return Ok(());
    }
    let Some((n, _)) = len else {
        return Err(uncovered_array(db, span));
    };
    let Some(dom) = subtype_range(db, elem, 0).or_else(|| type_range(db, elem_ty)) else {
        return Err(uncovered_array(db, span));
    };
    let values = u32::try_from(n).ok().and_then(|n| dom.len().checked_pow(n));
    let covered = if wild {
        // '-' matches every element value: enumerate when small.
        match values {
            Some(total) if total <= 4096 => {
                let k = dom.len() as i64;
                (0..total as i64).all(|mut v| {
                    let word: Vec<i64> = (0..n)
                        .map(|_| {
                            let d = dom.low() + v % k;
                            v /= k;
                            d
                        })
                        .collect();
                    patterns
                        .iter()
                        .any(|p| p.iter().zip(&word).all(|(&c, &w)| c == DONT_CARE || c == w))
                })
            }
            _ => patterns.iter().any(|p| p.iter().all(|&c| c == DONT_CARE)),
        }
    } else {
        values == Some(patterns.len() as u128)
    };
    if covered {
        Ok(())
    } else {
        Err(uncovered_array(db, span))
    }
}

fn uncovered_array(db: &mut Db, span: Span) -> Reported {
    db.error(
        Diagnostic::error(
            diag::CASE_COVERAGE,
            "case choices do not cover every value of the expression",
        )
        .with_primary(span, "not every value is covered")
        .with_help("add `when others =>`"),
    )
}

// ---------------------------------------------------------------------------
// Aggregates

/// Resolve an aggregate whose type comes from the context subtype `ctx`
/// (9.3.3). Builds `ExprKind::Aggregate` with the result bounds and, when
/// everything is locally static, the folded compact value.
pub fn resolve_aggregate(
    db: &mut Db,
    r: &mut dyn AggResolver,
    assocs: &[ast::ElementAssoc],
    ctx: SubtypeId,
    span: Span,
) -> Result<ExprId, Reported> {
    let ty = db.base(ctx);
    match &db.types[ty].kind {
        TypeKind::Error => Ok(db.poison.expr),
        TypeKind::Record { .. } => record_agg(db, r, ty, ctx, assocs, span),
        TypeKind::Array { indexes, elem } => {
            let (indexes, elem) = (indexes.to_vec(), *elem);
            let n_dims = indexes.len() as u32;
            // The element subtype with the context's element constraint applied (2008).
            let elem = db.element_subtype(ctx).unwrap_or(elem);
            let cx = ArrCx {
                ty,
                ctx,
                indexes,
                elem,
                ctx_dims: match &db.subtypes[ctx].constraint {
                    Constraint::Array { dims: Some(d), .. } => Some(d.to_vec()),
                    // `x'subtype`, a named return value's subtype: `x'range(k)` (#33).
                    &Constraint::FromObject {
                        expr,
                        elem_depth: 0,
                    } => Some(
                        (1..=n_dims)
                            .map(|dim| RangeC::Attr {
                                prefix: crate::hir::AttrPrefix::Expr(expr),
                                reverse: false,
                                dim,
                            })
                            .collect(),
                    ),
                    // `x'element` of an array of arrays with open element bounds: the
                    // part of the object's descriptor after the outer dimensions.
                    Constraint::FromObject { .. } => Some(
                        (1..=n_dims)
                            .map(|dim| RangeC::Attr {
                                prefix: crate::hir::AttrPrefix::Subtype(ctx),
                                reverse: false,
                                dim,
                            })
                            .collect(),
                    ),
                    _ => None,
                },
            };
            let built = array_agg(db, r, &cx, 0, assocs, span)?;
            Ok(built.expr)
        }
        _ => Err(db.error(
            Diagnostic::error(
                diag::AGG_NO_TYPE,
                format!("an aggregate cannot have type {}", type_name(db, ty)),
            )
            .with_primary(span, "aggregates are only of array and record types"),
        )),
    }
}

// -- records (9.3.3.2) --------------------------------------------------------

fn record_agg(
    db: &mut Db,
    r: &mut dyn AggResolver,
    ty: TypeId,
    ctx: SubtypeId,
    assocs: &[ast::ElementAssoc],
    span: Span,
) -> Result<ExprId, Reported> {
    let TypeKind::Record { elems } = &db.types[ty].kind else {
        unreachable!("record_agg on a record type")
    };
    let elems: Vec<(yavs_syntax::ast::Ident, SubtypeId)> =
        elems.iter().map(|e| (e.name, e.subtype)).collect();
    let mut owner: Vec<Option<(usize, Span)>> = vec![None; elems.len()];
    let mut covers: Vec<Vec<u32>> = vec![Vec::new(); assocs.len()];
    let mut failed = None;
    let mut seen_named = false;
    let mut next_pos = 0usize;
    for (ai, a) in assocs.iter().enumerate() {
        if a.choices.is_empty() {
            if seen_named {
                failed = Some(
                    db.error(
                        Diagnostic::error(
                            diag::AGG_NONSTATIC_CHOICE,
                            "positional element after a named one",
                        )
                        .with_primary(a.span, "positional")
                        .with_note("positional associations must come first (LRM 9.3.3.1)"),
                    ),
                );
                continue;
            }
            if next_pos >= elems.len() {
                failed = Some(
                    db.error(
                        Diagnostic::error(
                            diag::AGG_DUPLICATE,
                            format!(
                                "too many elements: {} has {} elements",
                                type_name(db, ty),
                                elems.len()
                            ),
                        )
                        .with_primary(a.span, "no element left for this value"),
                    ),
                );
                continue;
            }
            owner[next_pos] = Some((ai, a.span));
            covers[ai].push(next_pos as u32);
            next_pos += 1;
            continue;
        }
        seen_named = true;
        for c in &a.choices {
            match c {
                ast::Choice::Others(sp) => {
                    if ai + 1 != assocs.len() || a.choices.len() != 1 {
                        failed = Some(
                            db.error(
                                Diagnostic::error(
                                    diag::AGG_OTHERS,
                                    "`others` must be the last choice and stand alone",
                                )
                                .with_primary(*sp, "misplaced `others`"),
                            ),
                        );
                        continue;
                    }
                    for (i, o) in owner.iter_mut().enumerate() {
                        if o.is_none() {
                            *o = Some((ai, *sp));
                            covers[ai].push(i as u32);
                        }
                    }
                    if covers[ai].is_empty() {
                        failed = Some(
                            db.error(
                                Diagnostic::error(
                                    diag::AGG_OTHERS,
                                    "`others` does not stand for any element",
                                )
                                .with_primary(*sp, "every element already has a value")
                                .with_note(
                                    "`others` must cover at least one element (LRM 9.3.3.2)",
                                ),
                            ),
                        );
                    }
                }
                ast::Choice::Expr(ast::Expr {
                    kind:
                        ast::ExprKind::Name(ast::Name {
                            kind: ast::NameKind::Simple(id),
                            ..
                        }),
                    span: sp,
                }) => match elems.iter().position(|e| e.0.key == id.key) {
                    Some(i) => {
                        if let Some((_, first)) = owner[i] {
                            failed = Some(
                                db.error(
                                    Diagnostic::error(
                                        diag::AGG_DUPLICATE,
                                        format!("element `{}` is given twice", elems[i].0.text),
                                    )
                                    .with_primary(*sp, "given again here")
                                    .with_secondary(first, "first given here"),
                                ),
                            );
                        } else {
                            owner[i] = Some((ai, *sp));
                            covers[ai].push(i as u32);
                        }
                    }
                    None => {
                        let mut d = Diagnostic::error(
                            diag::NOT_DECLARED,
                            format!("{} has no element `{}`", type_name(db, ty), id.text),
                        )
                        .with_primary(*sp, "unknown element");
                        if let Some(h) = crate::suggest::suggest(
                            id.key.as_str(),
                            "element",
                            elems.iter().map(|e| (e.0.key.as_str(), e.0.text.as_str())),
                        ) {
                            d = d.with_help(h);
                        }
                        failed = Some(db.error(d));
                    }
                },
                ast::Choice::Expr(e) => {
                    failed = Some(
                        db.error(
                            Diagnostic::error(
                                diag::AGG_RECORD_CHOICE,
                                "a record aggregate choice must be an element name",
                            )
                            .with_primary(e.span, "not an element name"),
                        ),
                    );
                }
                ast::Choice::Range(_) => {
                    failed = Some(
                        db.error(
                            Diagnostic::error(
                                diag::AGG_RECORD_CHOICE,
                                "a record aggregate choice must be an element name",
                            )
                            .with_primary(a.span, "a range is not an element name"),
                        ),
                    );
                }
            }
        }
    }
    let missing: Vec<String> = owner
        .iter()
        .zip(&elems)
        .filter(|(o, _)| o.is_none())
        .map(|(_, e)| format!("`{}`", e.0.text))
        .collect();
    if !missing.is_empty() && failed.is_none() {
        failed = Some(
            db.error(
                Diagnostic::error(
                    diag::AGG_MISSING,
                    format!("missing value for {}", missing.join(", ")),
                )
                .with_primary(span, format!("in this aggregate of {}", type_name(db, ty))),
            ),
        );
    }
    // Elements sharing one value must be of one type (9.3.3.2).
    for (ai, cov) in covers.iter().enumerate() {
        if let Some(&first) = cov.first() {
            let t0 = db.base(elems[first as usize].1);
            if let Some(&bad) = cov.iter().find(|&&i| db.base(elems[i as usize].1) != t0) {
                failed = Some(
                    db.error(
                        Diagnostic::error(
                            diag::AGG_ELEMENT,
                            format!(
                                "elements `{}` and `{}` share one value but have different types",
                                elems[first as usize].0.text, elems[bad as usize].0.text
                            ),
                        )
                        .with_primary(assocs[ai].span, "one value for both"),
                    ),
                );
            }
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    let mut out = Vec::with_capacity(assocs.len());
    let mut values: Vec<Option<ConstValue>> = vec![None; elems.len()];
    let mut stat = Staticness::Locally;
    let mut failed = None;
    for (ai, a) in assocs.iter().enumerate() {
        // Each element's subtype has the context's record constraint on it (6.3). A value
        // shared by elements of different subtypes (`others => (others => '0')` for
        // vectors of different widths) is resolved once per subtype (9.3.3.2).
        let mut groups: Vec<(SubtypeId, Vec<u32>)> = Vec::new();
        for &i in &covers[ai] {
            let st = db.field_subtype(ctx, i);
            match groups.iter().position(|(g, _)| *g == st) {
                Some(k) => groups[k].1.push(i),
                None => groups.push((st, vec![i])),
            }
        }
        for (st, group) in groups {
            match sub_value(db, r, st, &a.value) {
                Ok(v) => {
                    stat = stat.min(db.exprs[v].stat).min(db.constraint_staticness(st));
                    for &i in &group {
                        values[i as usize] = db.value(v).cloned();
                    }
                    out.push(AggAssoc {
                        choices: group.iter().map(|&i| Choice::Element(i)).collect(),
                        value: v,
                        slice_piece: false,
                    });
                }
                Err(e) => failed = Some(e),
            }
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    let value = if stat == Staticness::Locally {
        values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .map(|v| ConstValue::Record(v.into()))
    } else {
        None
    };
    Ok(mk_expr(
        db,
        ExprKind::Aggregate {
            assocs: out.into(),
            bounds: AggBounds::Record,
        },
        ctx,
        span,
        stat,
        value,
    ))
}

// -- arrays (9.3.3.3) ---------------------------------------------------------

struct ArrCx {
    ty: TypeId,
    ctx: SubtypeId,
    indexes: Vec<SubtypeId>,
    elem: SubtypeId,
    ctx_dims: Option<Vec<RangeC>>,
}

/// One analyzed (sub-)aggregate.
struct Built {
    expr: ExprId,
    /// Static ranges of this dimension and the deeper ones.
    dims: Option<Vec<StaticRange>>,
    /// Folded elements, row-major runs.
    runs: Option<Vec<(u64, ConstValue)>>,
}

/// How one association's value is used.
enum Val {
    /// An element of the element type.
    Elem(ExprId),
    /// 2008: a value of the aggregate type, contributing its elements.
    Piece(ExprId),
    /// A sub-aggregate (or string literal) for the next dimension.
    Row(Built),
}

fn array_agg(
    db: &mut Db,
    r: &mut dyn AggResolver,
    cx: &ArrCx,
    dim: usize,
    assocs: &[ast::ElementAssoc],
    span: Span,
) -> Result<Built, Reported> {
    let ndims = cx.indexes.len();
    let index_st = cx.indexes[dim];
    let index_ty = db.base(index_st);
    let elem_ty = db.base(cx.elem);

    // Shape: positional first, no mixing with named, `others` last and alone.
    let mut positional = 0usize;
    let mut named = 0usize;
    for (ai, a) in assocs.iter().enumerate() {
        let others = a
            .choices
            .iter()
            .any(|c| matches!(c, ast::Choice::Others(_)));
        if others && (ai + 1 != assocs.len() || a.choices.len() != 1) {
            let sp = a
                .choices
                .iter()
                .find_map(|c| match c {
                    ast::Choice::Others(sp) => Some(*sp),
                    _ => None,
                })
                .unwrap_or(a.span);
            return Err(db.error(
                Diagnostic::error(
                    diag::AGG_OTHERS,
                    "`others` must be the last choice and stand alone",
                )
                .with_primary(sp, "misplaced `others`"),
            ));
        }
        if a.choices.is_empty() {
            if named > 0 {
                return Err(mixed(db, a.span));
            }
            positional += 1;
        } else if !others {
            if positional > 0 {
                return Err(mixed(db, a.span));
            }
            named += 1;
        }
    }
    let has_others = assocs.last().is_some_and(|a| {
        a.choices
            .iter()
            .any(|c| matches!(c, ast::Choice::Others(_)))
    });
    let ctx_dim = cx.ctx_dims.as_ref().map(|d| d[dim].clone());
    if has_others && ctx_dim.is_none() {
        return Err(db.error(
            Diagnostic::error(
                diag::AGG_OTHERS,
                "the bounds of an aggregate with `others` must come from the context",
            )
            .with_primary(span, "the context subtype is not constrained")
            .with_help(format!(
                "qualify the aggregate with a constrained subtype of {}",
                type_name(db, cx.ty)
            )),
        ));
    }

    if has_others && dim == 0 && db.relaxed_target == Some(cx.ctx) {
        db.report(
            Diagnostic::warning(
                diag::W_OTHERS_TARGET,
                "`others` takes the bounds of an unconstrained target",
            )
            .with_primary(span, "accepted by `--relaxed`")
            .with_note("LRM 9.3.3.3 needs a constrained subtype from the context here")
            .with_help("constrain the target's subtype, or qualify the aggregate with a type mark"),
        );
    }

    // One misplaced scalar in an n-dimensional aggregate is usually all of
    // them (`('0', '1')` for a 2-D type): report the first only.
    if dim + 1 < ndims
        && let Some(a) = assocs.iter().find(|a| !is_row(&a.value, dim + 2 == ndims))
    {
        return Err(not_a_row(db, cx, dim, a.value.span));
    }

    // Values and choices.
    let mut vals = Vec::with_capacity(assocs.len());
    let mut choices: Vec<Vec<Choice>> = Vec::with_capacity(assocs.len());
    let mut failed = None;
    for a in assocs {
        // A range choice, also as a subtype name (9.3.3.3).
        let piece_ok = a.choices.iter().all(|c| match c {
            ast::Choice::Range(_) => true,
            ast::Choice::Expr(e) => r.type_mark(db, e).is_some(),
            ast::Choice::Others(_) => false,
        });
        match resolve_value(db, r, cx, dim, &a.value, piece_ok) {
            Ok(v) => vals.push(v),
            Err(e) => {
                failed = Some(e);
                continue;
            }
        }
        match resolve_choices(db, r, &a.choices, index_ty) {
            Ok(c) => choices.push(c),
            Err(e) => {
                failed = Some(e);
                choices.push(Vec::new());
            }
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }

    let mut stat = Staticness::Locally;
    for v in &vals {
        let e = match v {
            Val::Elem(e) | Val::Piece(e) => *e,
            Val::Row(b) => b.expr,
        };
        stat = stat.min(db.exprs[e].stat);
    }

    // A slice piece under a single-value choice needs an element.
    for (v, cs) in vals.iter().zip(&choices) {
        if let Val::Piece(e) = v
            && cs.iter().any(|c| matches!(c, Choice::Expr(_)))
        {
            return Err(db.error(
                Diagnostic::error(
                    diag::AGG_ELEMENT,
                    format!(
                        "expected an element of type {}, found a value of the aggregate type",
                        type_name(db, elem_ty)
                    ),
                )
                .with_primary(db.exprs[*e].span, "an array value needs a range choice"),
            ));
        }
    }

    // Named choices: staticness, overlap, contiguity, range.
    let named_items = if named > 0 || has_others {
        let refs: Vec<&[Choice]> = choices.iter().map(|c| c.as_slice()).collect();
        let n_choices: usize = refs.iter().map(|c| c.len()).sum();
        let single = named == 1 && !has_others && n_choices == 1;
        for c in refs.iter().flat_map(|c| c.iter()) {
            let s = match c {
                Choice::Expr(e) => db.exprs[*e].stat,
                Choice::Range(rc) => range_stat(db, rc),
                _ => Staticness::Locally,
            };
            stat = stat.min(s);
            // A null range choice, too, only as the single choice (1993 7.3.2.2, #812); in later
            // revisions sim1 and sim2 accept it, so yavs does too.
            let null = db.file_rev() == crate::LangRev::Vhdl1993
                && matches!(c, Choice::Range(rc) if range_value(db, rc).is_some_and(|r| r.is_null()));
            if null && !single {
                let sp = choice_span(db, c, span);
                return Err(db.error(
                    Diagnostic::error(
                        diag::AGG_NONSTATIC_CHOICE,
                        "a null range choice must be the only choice",
                    )
                    .with_primary(sp, "null range")
                    .with_note(
                        "only an aggregate with a single association and a single choice may use a null range choice (LRM 9.3.3.3)",
                    ),
                ));
            }
            if s < Staticness::Locally && !single {
                let sp = choice_span(db, c, span);
                return Err(db.error(
                    Diagnostic::error(
                        diag::AGG_NONSTATIC_CHOICE,
                        "a choice that is not locally static must be the only choice",
                    )
                    .with_primary(sp, "not locally static")
                    .with_note(
                        "only an aggregate with a single association and a single choice may use a non-static choice (LRM 9.3.3.3)",
                    ),
                ));
            }
        }
        match fold_items(db, &mut Folded, &refs, false, span)? {
            Some(items) => {
                let (cov, within) = if has_others {
                    (
                        Coverage::None,
                        ctx_dim.as_ref().and_then(|d| range_value(db, d)),
                    )
                } else {
                    (Coverage::Contiguous, subtype_range(db, index_st, 0))
                };
                // Range choices with an array value: lengths must agree.
                for (v, cs) in vals.iter().zip(&choices) {
                    if let (Val::Piece(e), [Choice::Range(rc)]) = (v, cs.as_slice())
                        && let (Some(rr), Some(n)) = (range_value(db, rc), piece_len(db, *e))
                        && rr.len() != n
                    {
                        let sp = db.exprs[*e].span;
                        return Err(db.error(
                            Diagnostic::error(
                                diag::LENGTH_MISMATCH,
                                format!(
                                    "the choice covers {} elements, the value has {n}",
                                    rr.len()
                                ),
                            )
                            .with_primary(sp, "length differs from the choice"),
                        ));
                    }
                }
                Some(check_items(
                    db,
                    items.clone(),
                    refs.len(),
                    index_ty,
                    cov,
                    within,
                    Ctx::Aggregate,
                    span,
                )?)
                .map(|_| items)
            }
            None => None,
        }
    } else {
        Some(Vec::new())
    };

    // Deeper dimensions: all rows must have the same static shape.
    let mut deeper: Option<Vec<StaticRange>> = None;
    let mut deeper_known = dim + 1 == ndims;
    if dim + 1 < ndims {
        deeper_known = true;
        let mut first: Option<(Vec<StaticRange>, Span)> = None;
        for v in &vals {
            let Val::Row(b) = v else { continue };
            match (&b.dims, &first) {
                (Some(d), Some((f, fsp))) => {
                    let lens: Vec<u128> = d.iter().map(|r| r.len()).collect();
                    let flens: Vec<u128> = f.iter().map(|r| r.len()).collect();
                    if lens != flens {
                        let sp = db.exprs[b.expr].span;
                        let fsp = *fsp;
                        return Err(db.error(
                            Diagnostic::error(
                                diag::LENGTH_MISMATCH,
                                "the sub-aggregates of a multidimensional aggregate differ in length",
                            )
                            .with_primary(sp, format!("{} elements", lens[0]))
                            .with_secondary(fsp, format!("{} elements", flens[0])),
                        ));
                    }
                }
                (Some(d), None) => first = Some((d.clone(), db.exprs[b.expr].span)),
                (None, _) => deeper_known = false,
            }
        }
        deeper = first.map(|f| f.0);
        if deeper.is_none() {
            deeper_known = false;
        }
    }

    // Bounds of this dimension.
    let index_range = subtype_range(db, index_st, 0);
    let dir_of = |db: &Db, rc: &Option<RangeC>| -> Direction {
        match rc {
            Some(RangeC::Explicit { dir, .. }) => *dir,
            Some(rc) => range_value(db, rc).map_or(Direction::To, |r| r.dir),
            None => index_range.map_or(Direction::To, |r| r.dir),
        }
    };
    // 2008 pieces under range choices: without a constrained context the
    // aggregate takes their direction, and they must agree (the reference
    // simulators; lrm-checks L84).
    let mut piece_dir: Option<(Direction, Span)> = None;
    for (v, cs) in vals.iter().zip(&choices) {
        if let (Val::Piece(_), [c @ Choice::Range(rc)]) = (v, cs.as_slice()) {
            let d = dir_of(db, &Some(rc.clone()));
            let sp = choice_span(db, c, span);
            match piece_dir {
                Some((first, fsp)) if first != d => {
                    return Err(db.error(
                        Diagnostic::error(
                            diag::AGG_PIECE_DIRECTION,
                            "the range choices of array values in this aggregate have different directions",
                        )
                        .with_primary(sp, "direction differs")
                        .with_secondary(fsp, "first range choice with an array value")
                        .with_note(
                            "without a constrained context the aggregate takes the direction of these choices",
                        ),
                    ));
                }
                Some(_) => {}
                None => piece_dir = Some((d, sp)),
            }
        }
    }
    let piece_dir = piece_dir.map(|p| p.0);
    let (bounds, this): (AggBounds, Option<StaticRange>) = if has_others {
        let rc = ctx_dim.as_ref().expect("checked above");
        (AggBounds::Context, range_value(db, rc))
    } else if named > 0 {
        let dir = match (&ctx_dim, piece_dir) {
            (None, Some(d)) => d,
            _ => dir_of(db, &ctx_dim),
        };
        match &named_items {
            Some(items) => {
                let lo = items.iter().map(|i| i.lo).min().expect("named choices");
                let hi = items.iter().map(|i| i.hi).max().expect("named choices");
                let lo_e = mk_const(db, pos_value(db, index_ty, lo), index_st, span);
                let hi_e = mk_const(db, pos_value(db, index_ty, hi), index_st, span);
                let r = match dir {
                    Direction::To => StaticRange::new(lo, dir, hi),
                    Direction::Downto => StaticRange::new(hi, dir, lo),
                };
                (
                    AggBounds::Named {
                        low: lo_e,
                        high: hi_e,
                    },
                    Some(r),
                )
            }
            None => {
                // A single non-static (or deferred) choice.
                let c = &choices.iter().find(|c| !c.is_empty()).expect("named")[0];
                let (low, high) = choice_low_high(db, c, index_st, span);
                (AggBounds::Named { low, high }, None)
            }
        }
    } else {
        let mut count: Option<u128> = Some(0);
        let mut concat = false;
        for v in &vals {
            let n = match v {
                Val::Elem(_) | Val::Row(_) => Some(1),
                Val::Piece(e) => {
                    concat = true;
                    piece_len(db, *e)
                }
            };
            count = count.zip(n).map(|(a, b)| a + b);
        }
        if let Some(rc) = &ctx_dim {
            let r = range_value(db, rc);
            if let (Some(r), Some(n)) = (r, count)
                && r.len() != n
            {
                return Err(db.error(
                    Diagnostic::error(
                        diag::LENGTH_MISMATCH,
                        format!(
                            "the aggregate has {n} elements, the context subtype has {}",
                            r.len()
                        ),
                    )
                    .with_primary(span, format!("{n} elements")),
                ));
            }
            (AggBounds::Context, r)
        } else {
            let r = match (index_range, count) {
                (Some(ir), Some(n)) => {
                    let left = ir.left;
                    let step = i128::try_from(n).ok().filter(|&n| n > 0).map(|n| n - 1);
                    let right = step.and_then(|s| match ir.dir {
                        Direction::To => i64::try_from(i128::from(left) + s).ok(),
                        Direction::Downto => i64::try_from(i128::from(left) - s).ok(),
                    });
                    match right {
                        Some(right) if ir.contains(right) || n == 0 => {
                            Some(StaticRange::new(left, ir.dir, right))
                        }
                        Some(_) | None if n > 0 => {
                            return Err(db.error(
                                Diagnostic::error(
                                    diag::OUT_OF_SUBTYPE,
                                    format!(
                                        "the aggregate has {n} elements, more than its index subtype holds"
                                    ),
                                )
                                .with_primary(span, format!("{n} elements"))
                                .with_note(format!(
                                    "the index subtype is {}",
                                    show_interval(db, index_ty, ir.low(), ir.high())
                                )),
                            ));
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            if concat {
                (AggBounds::Concat, r)
            } else {
                let n = i64::try_from(count.expect("no pieces: every element counts 1"))
                    .expect("element count fits i64");
                let count_st = db
                    .well_known
                    .universal_integer
                    .map_or(index_st, |t| db.types[t].first_subtype);
                let c = mk_const(db, ConstValue::Int(n), count_st, span);
                (AggBounds::Positional { count: c }, r)
            }
        }
    };
    if this.is_none() {
        stat = stat.min(Staticness::Globally);
    }

    let dims = match (this, deeper_known) {
        (Some(t), true) => {
            let mut d = vec![t];
            d.extend(deeper.clone().unwrap_or_default());
            Some(d)
        }
        _ => None,
    };

    // Fold (rule 12). Elements of an unconstrained composite subtype must all have the same
    // shape (9.3.3.3): a folded value could not say otherwise, so the run-time check decides.
    let runs = if stat == Staticness::Locally {
        match (&dims, &named_items) {
            (Some(d), Some(items)) => fold_runs(db, &vals, &choices, items, d[0], has_others)
                .filter(|runs| runs.windows(2).all(|w| same_shape(&w[0].1, &w[1].1))),
            _ => None,
        }
    } else {
        None
    };

    // Result subtype: refined only for the outermost aggregate (see module doc).
    let subtype = if dim == 0 && !matches!(bounds, AggBounds::Context) {
        result_subtype(db, cx, &dims, &bounds, piece_dir, span)
    } else {
        cx.ctx
    };
    let value = match (&runs, &dims) {
        (Some(runs), Some(d)) if dim == 0 => Some(ConstValue::Array(array_const(
            db,
            elem_ty,
            d.clone(),
            runs.clone(),
        ))),
        _ => None,
    };
    let assocs_hir: Vec<AggAssoc> = vals
        .into_iter()
        .zip(choices)
        .map(|(v, cs)| {
            let (value, slice_piece) = match v {
                Val::Elem(e) => (e, false),
                Val::Piece(e) => (e, true),
                Val::Row(b) => (b.expr, false),
            };
            AggAssoc {
                choices: cs.into(),
                value,
                slice_piece,
            }
        })
        .collect();
    let expr = mk_expr(
        db,
        ExprKind::Aggregate {
            assocs: assocs_hir.into(),
            bounds,
        },
        subtype,
        span,
        stat,
        value,
    );
    Ok(Built { expr, dims, runs })
}

/// Whether two element values have the same lengths in every array they contain (an inner
/// array is uniform: it folded under the same rule).
fn same_shape(a: &ConstValue, b: &ConstValue) -> bool {
    let lens = |bs: &[(i64, Direction, i64)]| {
        bs.iter()
            .map(|&(l, d, r)| StaticRange::new(l, d, r).len())
            .collect::<Vec<_>>()
    };
    match (a, b) {
        (ConstValue::Array(x), ConstValue::Array(y)) => {
            lens(&x.bounds) == lens(&y.bounds)
                && match (elem_at(x), elem_at(y)) {
                    (Some(p), Some(q)) => same_shape(&p, &q),
                    _ => true,
                }
        }
        (ConstValue::Record(x), ConstValue::Record(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| same_shape(p, q))
        }
        _ => true,
    }
}

/// The first element of an array constant, when it is composite.
fn elem_at(a: &ArrayConst) -> Option<ConstValue> {
    let v = match &a.elems {
        Elems::Runs(rs) => rs.first()?.1.clone(),
        Elems::Bytes(_) => return None,
    };
    matches!(v, ConstValue::Array(_) | ConstValue::Record(_)).then_some(v)
}

fn mixed(db: &mut Db, span: Span) -> Reported {
    db.error(
        Diagnostic::error(
            diag::AGG_NONSTATIC_CHOICE,
            "named and positional associations are mixed in an array aggregate",
        )
        .with_primary(span, "this association")
        .with_note("only a final `others` may follow positional associations (LRM 9.3.3.3)"),
    )
}

/// The `ConstValue` of index position `p` of type `ty`.
fn pos_value(db: &Db, ty: TypeId, p: i64) -> ConstValue {
    match db.types[ty].kind {
        TypeKind::Enum { .. } => ConstValue::Enum(p as u32),
        _ => ConstValue::Int(p),
    }
}

/// Static length of a slice piece.
fn piece_len(db: &Db, e: ExprId) -> Option<u128> {
    if let Some(ConstValue::Array(a)) = db.value(e) {
        return Some(u128::from(array_len(&a.bounds[..1])));
    }
    subtype_range(db, db.exprs[e].subtype, 0).map(|r| r.len())
}

/// Low and high bound expressions of a single (possibly non-static) choice.
fn choice_low_high(db: &mut Db, c: &Choice, index_st: SubtypeId, span: Span) -> (ExprId, ExprId) {
    match c {
        Choice::Expr(e) => (*e, *e),
        Choice::Range(RangeC::Explicit { left, dir, right }) => match dir {
            Direction::To => (*left, *right),
            Direction::Downto => (*right, *left),
        },
        Choice::Range(RangeC::Attr {
            prefix,
            reverse: _,
            dim,
        }) => {
            let dimc = mk_const(
                db,
                ConstValue::Int(i64::from((*dim).max(1))),
                index_st,
                span,
            );
            let attr = |db: &mut Db, a| {
                mk_expr(
                    db,
                    ExprKind::Attr {
                        attr: a,
                        prefix: *prefix,
                        args: Box::new([dimc]),
                    },
                    index_st,
                    span,
                    Staticness::Globally,
                    None,
                )
            };
            let low = attr(db, crate::attr::PredefAttr::Low);
            let high = attr(db, crate::attr::PredefAttr::High);
            (low, high)
        }
        Choice::Others | Choice::Element(_) => {
            unreachable!("a single named choice is a value or a range")
        }
    }
}

fn resolve_value(
    db: &mut Db,
    r: &mut dyn AggResolver,
    cx: &ArrCx,
    dim: usize,
    value: &ast::Expr,
    piece_ok: bool,
) -> Result<Val, Reported> {
    let ndims = cx.indexes.len();
    let elem_ty = db.base(cx.elem);
    if dim + 1 < ndims {
        return match &value.kind {
            ast::ExprKind::Aggregate(sub) => {
                array_agg(db, r, cx, dim + 1, sub, value.span).map(Val::Row)
            }
            ast::ExprKind::Literal(ast::Literal {
                kind: ast::LiteralKind::String(s),
                ..
            }) if dim + 2 == ndims => string_row(db, cx, s.as_str(), value.span).map(Val::Row),
            _ => Err(not_a_row(db, cx, dim, value.span)),
        };
    }
    let c = r.cands(db, value);
    if is_empty(&c) {
        // Let the resolver report what is wrong with the expression.
        return elem_value(db, r, cx, value).map(Val::Elem);
    }
    let as_elem = fits(db, &c, elem_ty);
    // The aggregate's own type is a candidate only for a positional
    // association or a discrete range choice (9.3.3.3, lrm-checks L8).
    let as_piece = ndims == 1 && piece_ok && fits(db, &c, cx.ty);
    match (as_elem, as_piece) {
        (true, false) => elem_value(db, r, cx, value).map(Val::Elem),
        // 1993 7.3.2.2: every element is a value of the element type; array pieces are 2008
        // (9.3.3.3, #814).
        (false, true) if db.file_rev() == crate::LangRev::Vhdl1993 => Err(db.error(
            Diagnostic::error(
                diag::NOT_IN_REVISION,
                format!(
                    "an element of type {} is expected: an array value as a part of an aggregate is not allowed in VHDL-1993",
                    type_name(db, elem_ty)
                ),
            )
            .with_primary(value.span, "a value of the aggregate's own type")
            .with_note("LRM 1993 7.3.2.2: each element association gives one element; slices and array values as parts of an aggregate are VHDL-2008 (LRM 2008 9.3.3.3)"),
        )),
        (false, true) => r.resolve(db, value, Expected::Type(cx.ty)).map(Val::Piece),
        // No LRM tie-break (lrm-checks L8): the element type wins, as in sim1.
        (true, true) => elem_value(db, r, cx, value).map(Val::Elem),
        (false, false) => {
            // `others` gives an element value, never a piece (9.3.3.3).
            if ndims == 1 && !piece_ok && fits(db, &c, cx.ty) {
                return Err(db.error(
                    Diagnostic::error(
                        diag::AGG_ELEMENT,
                        format!(
                            "expected an element of type {}, found a value of the aggregate type",
                            type_name(db, elem_ty)
                        ),
                    )
                    .with_primary(value.span, "an array value needs a range choice")
                    .with_note("an association with `others` gives an element value (LRM 9.3.3.3)"),
                ));
            }
            let piece = if ndims == 1 && piece_ok {
                format!(" or {}", type_name(db, cx.ty))
            } else {
                String::new()
            };
            Err(db.error(
                Diagnostic::error(
                    diag::AGG_ELEMENT,
                    format!(
                        "expected an element of type {}{piece}",
                        type_name(db, elem_ty)
                    ),
                )
                .with_primary(
                    value.span,
                    "neither the element type nor the aggregate type",
                ),
            ))
        }
    }
}

/// Can `e` be an element of dimension `dim < n-1` of an n-dimensional
/// aggregate: an aggregate, or a string literal for the last dimension?
fn is_row(e: &ast::Expr, last: bool) -> bool {
    match &e.kind {
        ast::ExprKind::Aggregate(_) => true,
        ast::ExprKind::Literal(ast::Literal {
            kind: ast::LiteralKind::String(_),
            ..
        }) => last,
        _ => false,
    }
}

fn not_a_row(db: &mut Db, cx: &ArrCx, dim: usize, span: Span) -> Reported {
    db.error(
        Diagnostic::error(
            diag::AGG_ELEMENT,
            format!(
                "expected an aggregate for dimension {} of {}",
                dim + 2,
                type_name(db, cx.ty)
            ),
        )
        .with_primary(span, "not an aggregate")
        .with_note(
            "each element of an n-dimensional aggregate is an (n-1)-dimensional aggregate or, for the last dimension, a string literal (LRM 9.3.3.3)",
        ),
    )
}

/// An element value. A nested aggregate is analyzed here with the element
/// subtype as its context, since `Expected` carries only a type and an
/// `others` inside needs the constrained element subtype.
fn elem_value(
    db: &mut Db,
    r: &mut dyn AggResolver,
    cx: &ArrCx,
    value: &ast::Expr,
) -> Result<ExprId, Reported> {
    sub_value(db, r, cx.elem, value)
}

/// Resolve `value` for an element of subtype `st`.
fn sub_value(
    db: &mut Db,
    r: &mut dyn AggResolver,
    st: SubtypeId,
    value: &ast::Expr,
) -> Result<ExprId, Reported> {
    match &value.kind {
        ast::ExprKind::Aggregate(sub) => resolve_aggregate(db, r, sub, st, value.span),
        _ => {
            let ty = db.base(st);
            r.resolve(db, value, Expected::Type(ty))
        }
    }
}

/// A string literal standing for the last dimension of a multidimensional
/// aggregate of a character element type.
fn string_row(db: &mut Db, cx: &ArrCx, s: &str, span: Span) -> Result<Built, Reported> {
    let elem_ty = db.base(cx.elem);
    let dim = cx.indexes.len() - 1;
    let TypeKind::Enum { lits } = &db.types[elem_ty].kind else {
        return Err(db.error(
            Diagnostic::error(
                diag::AGG_ELEMENT,
                format!(
                    "a string literal needs a character element type, found {}",
                    type_name(db, elem_ty)
                ),
            )
            .with_primary(span, "string literal"),
        ));
    };
    let mut runs = Vec::new();
    for ch in s.chars() {
        match lits
            .iter()
            .position(|&l| db.decls[l].name == Designator::Char(ch))
        {
            Some(p) => runs.push((1, ConstValue::Enum(p as u32))),
            None => {
                return Err(db.error(
                    Diagnostic::error(
                        diag::CHAR_NOT_IN_TYPE,
                        format!("'{ch}' is not a literal of {}", type_name(db, elem_ty)),
                    )
                    .with_primary(span, "in this string literal"),
                ));
            }
        }
    }
    let n = runs.len() as i64;
    let index_st = cx.indexes[dim];
    let r = match &cx.ctx_dims {
        Some(d) => range_value(db, &d[dim]),
        None => subtype_range(db, index_st, 0).and_then(|ir| {
            let right = match ir.dir {
                Direction::To => ir.left.checked_add(n - 1),
                Direction::Downto => ir.left.checked_sub(n - 1),
            }?;
            Some(StaticRange::new(ir.left, ir.dir, right))
        }),
    };
    if let Some(r) = r
        && r.len() != n as u128
    {
        return Err(db.error(
            Diagnostic::error(
                diag::LENGTH_MISMATCH,
                format!("the string has {n} elements, the dimension has {}", r.len()),
            )
            .with_primary(span, format!("{n} elements")),
        ));
    }
    let value = r.map(|r| ConstValue::Array(array_const(db, elem_ty, vec![r], runs.clone())));
    let c = match value {
        Some(v) => db.add_const(v),
        None => db.add_const(ConstValue::Null),
    };
    let expr = db.add_expr(Expr {
        kind: ExprKind::Const(c),
        subtype: cx.ctx,
        span,
        stat: Staticness::Locally,
        value: r.map(|_| c),
    });
    Ok(Built {
        expr,
        dims: r.map(|r| vec![r]),
        runs: Some(runs),
    })
}

/// Row-major runs of this dimension from folded values and choices.
fn fold_runs(
    db: &Db,
    vals: &[Val],
    choices: &[Vec<Choice>],
    items: &[Item],
    this: StaticRange,
    has_others: bool,
) -> Option<Vec<(u64, ConstValue)>> {
    // Elements that are arrays of different lengths (an unconstrained element type) are not
    // folded: the run-time element check reports the mismatch (LRM 9.3.3.3).
    let mut shape: Option<Vec<i128>> = None;
    for v in vals {
        if let Val::Elem(e) = v
            && let Some(ConstValue::Array(a)) = db.value(*e)
        {
            let lens = a
                .bounds
                .iter()
                .map(|&(l, d, r)| {
                    let (lo, hi) = if d == Direction::To { (l, r) } else { (r, l) };
                    (i128::from(hi) - i128::from(lo) + 1).max(0)
                })
                .collect::<Vec<_>>();
            if shape.get_or_insert_with(|| lens.clone()) != &lens {
                return None;
            }
        }
    }
    // The runs one index position contributes, or a piece's whole runs.
    let row = |v: &Val| -> Option<Vec<(u64, ConstValue)>> {
        match v {
            Val::Elem(e) => Some(vec![(1, db.value(*e)?.clone())]),
            Val::Piece(e) => match db.value(*e)? {
                ConstValue::Array(a) => Some(elem_runs(a)),
                _ => None,
            },
            Val::Row(b) => b.runs.clone(),
        }
    };
    let positional = choices
        .iter()
        .all(|c| c.iter().all(|c| matches!(c, Choice::Others)));
    if positional {
        let mut out = Vec::new();
        let mut n = 0u64;
        let k = if has_others {
            vals.len() - 1
        } else {
            vals.len()
        };
        for v in &vals[..k] {
            let r = row(v)?;
            n += if matches!(v, Val::Piece(_)) {
                // Pieces exist only in 1-D aggregates: one element per position.
                r.iter().map(|r| r.0).sum::<u64>()
            } else {
                1
            };
            out.extend(r);
            if out.len() > RUNS_MAX {
                return None;
            }
        }
        if has_others {
            let fill = row(vals.last()?)?;
            let total = this.len() as u64;
            for _ in n..total {
                if let [(c, v)] = fill.as_slice() {
                    out.push((c * (total - n), v.clone()));
                    break;
                }
                out.extend(fill.iter().cloned());
                if out.len() > RUNS_MAX {
                    return None;
                }
            }
        }
        return Some(out);
    }
    // Segments: (offset, positions, value runs, is_piece).
    let mut segs: Vec<(u64, u64, Runs, bool)> = Vec::new();
    for it in items {
        if it.lo > it.hi {
            continue;
        }
        let v = &vals[it.alt];
        let off = this.offset(it.lo).min(this.offset(it.hi));
        let n = (i128::from(it.hi) - i128::from(it.lo) + 1) as u64;
        segs.push((off, n, row(v)?, matches!(v, Val::Piece(_))));
    }
    segs.sort_by_key(|s| s.0);
    let others_row = if has_others {
        Some(row(vals.last()?)?)
    } else {
        None
    };
    let total = this.len() as u64;
    let mut out: Vec<(u64, ConstValue)> = Vec::new();
    let mut at = 0u64;
    let emit = |out: &mut Vec<(u64, ConstValue)>, n: u64, r: &[(u64, ConstValue)]| -> bool {
        if n == 0 {
            return true;
        }
        if let [(c, v)] = r {
            out.push((c * n, v.clone()));
        } else {
            for _ in 0..n {
                out.extend(r.iter().cloned());
                if out.len() > RUNS_MAX {
                    return false;
                }
            }
        }
        true
    };
    for (off, n, r, piece) in &segs {
        if *off > at {
            let fill = others_row.as_ref()?;
            if !emit(&mut out, off - at, fill) {
                return None;
            }
        }
        if *piece {
            // A slice piece follows the aggregate's direction, so it reads
            // left to right in offset order.
            out.extend(r.iter().cloned());
        } else if !emit(&mut out, *n, r) {
            return None;
        }
        at = off + n;
    }
    if at < total {
        let fill = others_row.as_ref()?;
        if !emit(&mut out, total - at, fill) {
            return None;
        }
    }
    Some(out)
}

/// The result subtype of an outermost aggregate whose bounds do not come
/// from the context: the context subtype refined with the computed bounds.
fn result_subtype(
    db: &mut Db,
    cx: &ArrCx,
    dims: &Option<Vec<StaticRange>>,
    bounds: &AggBounds,
    piece_dir: Option<Direction>,
    span: Span,
) -> SubtypeId {
    let elem = match &db.subtypes[cx.ctx].constraint {
        Constraint::Array { elem, .. } => elem.clone(),
        _ => Box::new(Constraint::None),
    };
    let ranges: Option<Vec<RangeC>> = match (dims, bounds) {
        (Some(d), _) => Some(
            d.iter()
                .enumerate()
                .map(|(k, r)| {
                    let ist = cx.indexes[k];
                    let ity = db.base(ist);
                    let left = mk_const(db, pos_value(db, ity, r.left), ist, span);
                    let right = mk_const(db, pos_value(db, ity, r.right), ist, span);
                    RangeC::Explicit {
                        left,
                        dir: r.dir,
                        right,
                    }
                })
                .collect(),
        ),
        // 1-D named aggregate with one non-static choice.
        (None, AggBounds::Named { low, high }) if cx.indexes.len() == 1 => {
            let dir = piece_dir.unwrap_or_else(|| {
                subtype_range(db, cx.indexes[0], 0).map_or(Direction::To, |r| r.dir)
            });
            let (left, right) = match dir {
                Direction::To => (*low, *high),
                Direction::Downto => (*high, *low),
            };
            Some(vec![RangeC::Explicit { left, dir, right }])
        }
        // ponytail (#42): non-static positional and concatenation bounds stay in
        // `AggBounds` only; building `S'left + count - 1` needs the resolved
        // `"+"` of the index type.
        _ => None,
    };
    match ranges {
        Some(r) => db.subtypes.push(SubtypeDef {
            base: cx.ty,
            name: None,
            parent: Some(cx.ctx),
            constraint: Constraint::Array {
                dims: Some(r.into()),
                elem,
            },
            resolution: db.subtypes[cx.ctx].resolution.clone(),
            span,
        }),
        None => cx.ctx,
    }
}

#[cfg(test)]
#[path = "aggregate_tests.rs"]
pub(crate) mod tests;
