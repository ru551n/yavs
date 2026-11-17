//! Top-down selection and HIR construction (sem.md 3.2, 3.6, 3.7).

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast;

use super::{CandSet, Expected, Interp, Via};
use crate::db::Db;
use crate::decl::operator_key;
use crate::diag;
use crate::hir::{Actual, Arg, Expr, ExprKind};
use crate::ids::{DeclId, ExprId, ScopeId, TypeId};
use crate::scope::Visible;
use crate::stat::Staticness;
use crate::types::TypeKind;

use super::cands::{binary_symbol, unary_symbol};
use super::names::NameRes;

/// Most candidate labels on one "no matching overload" diagnostic (8.3).
const MAX_CANDIDATE_LABELS: usize = 8;

/// What `resolve_head` leaves to the loop in `resolve`.
enum Head<'a> {
    Done(ExprId),
    Left(Frame<'a>),
}

impl From<ExprId> for Head<'_> {
    fn from(id: ExprId) -> Self {
        Head::Done(id)
    }
}

/// A binary operator waiting for its left operand.
struct Frame<'a> {
    left: &'a ast::Expr,
    left_st: crate::ids::SubtypeId,
    /// The context `left` replaced, when it is the operand of a declared operator.
    saved_top: Option<Option<(usize, crate::ids::SubtypeId)>>,
    operands: [&'a ast::Expr; 2],
    op: OpHead,
    span: Span,
    i: Interp,
    exp: Expected,
}

/// The callee of an operator call and what its arguments need.
struct OpHead {
    callee: DeclId,
    pre: rustc_hash::FxHashMap<usize, ExprId>,
    formals: smallvec::SmallVec<[crate::ids::SubtypeId; 2]>,
    ret: crate::ids::SubtypeId,
    implicit: bool,
    stat: Staticness,
}

impl Db {
    /// Resolve one complete context (LRM 12.5) where the value is not required
    /// at analysis time: static overflow is warning `W1401` (Q5).
    pub(crate) fn expr(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected) -> ExprId {
        self.complete_context(scope, e, exp, false)
    }

    /// E1360: `e` reads a port of mode `linkage` (6.5.6.3). Attribute prefixes are not
    /// reads; with `target`, neither is the object an assignment target names.
    pub(crate) fn check_linkage_reads(
        &mut self,
        e: ExprId,
        target: bool,
    ) -> Result<(), crate::Reported> {
        // Linkage ports are rare: look for one first, and work out the exceptions only then.
        let mut linkage: smallvec::SmallVec<[ExprId; 2]> = smallvec::SmallVec::new();
        self.walk_expr(e, &mut |x| {
            if let ExprKind::Obj(d) = self.exprs[x].kind
                && let crate::decl::DeclKind::Object(o) = &self.decls[d].kind
                && o.mode == Some(crate::hir::Mode::Linkage)
            {
                linkage.push(x);
            }
        });
        if linkage.is_empty() {
            return Ok(());
        }
        let mut all = Vec::new();
        self.walk_expr(e, &mut |x| all.push(x));
        let mut skip = Vec::new();
        for &x in &all {
            if let ExprKind::Attr {
                prefix: crate::hir::AttrPrefix::Expr(p),
                ..
            } = self.exprs[x].kind
            {
                self.walk_expr(p, &mut |y| skip.push(y));
            }
        }
        if target {
            let mut root = e;
            while let ExprKind::Index { prefix, .. }
            | ExprKind::Slice { prefix, .. }
            | ExprKind::Field { prefix, .. } = self.exprs[root].kind
            {
                root = prefix;
            }
            skip.push(root);
        }
        for x in all {
            if skip.contains(&x) {
                continue;
            }
            if let ExprKind::Obj(d) = self.exprs[x].kind
                && let crate::decl::DeclKind::Object(o) = &self.decls[d].kind
                && o.mode == Some(crate::hir::Mode::Linkage)
            {
                let name = self.decls[d].name;
                return Err(self.error(
                    Diagnostic::error(
                        diag::CLASS_MISUSE,
                        format!("cannot read port `{name}` of mode `linkage`"),
                    )
                    .with_primary(self.exprs[x].span, "read here")
                    .with_note("a `linkage` port can only be associated with another `linkage` port (LRM 6.5.6.3)"),
                ));
            }
        }
        Ok(())
    }

    /// The object `e` names is not read by its context (an assignment target, the actual of
    /// an `out` formal, the prefix of an attribute that does not read the value, an alias
    /// target): the reference to an `out` port or parameter at its root is not a read
    /// (LRM 1993 4.3.2). The indices of an indexed target and the arguments of a call stay
    /// reads.
    pub(crate) fn not_read(&mut self, e: ExprId) {
        if self.out_reads.is_empty() {
            return;
        }
        let mut x = e;
        loop {
            match &self.exprs[x].kind {
                ExprKind::Index { prefix, .. }
                | ExprKind::Slice { prefix, .. }
                | ExprKind::Field { prefix, .. } => x = *prefix,
                // `conv(port)` as the actual of an `out` formal converts the formal's value.
                ExprKind::Conv { operand, .. }
                | ExprKind::Qualified { operand, .. }
                | ExprKind::ImplicitConv { operand } => x = *operand,
                ExprKind::Call { args, .. } if args.len() == 1 => match args[0].actual {
                    Actual::Expr(a) => x = a,
                    _ => break,
                },
                ExprKind::Aggregate { assocs, .. } => {
                    let parts: smallvec::SmallVec<[ExprId; 4]> =
                        assocs.iter().map(|a| a.value).collect();
                    for p in parts {
                        self.not_read(p);
                    }
                    return;
                }
                _ => break,
            }
        }
        self.out_reads.retain(|&p| p != x);
    }

    /// `not_read` for the actual of an `out` formal in a call or a map.
    pub(crate) fn not_read_actual(&mut self, a: &Actual) {
        if self.out_reads.is_empty() {
            return;
        }
        match a {
            Actual::Expr(x) | Actual::Converted { actual: x, .. } => self.not_read(*x),
            Actual::Individual(parts) => {
                let xs: smallvec::SmallVec<[ExprId; 4]> = parts.iter().map(|p| p.1).collect();
                for x in xs {
                    self.not_read(x);
                }
            }
            _ => {}
        }
    }

    /// `E1360`: what is left of `out_reads` at the end of a unit are reads of `out` ports and
    /// parameters in a VHDL-1993 file (LRM 1993 4.3.2: the value of an `out` interface object
    /// cannot be read; VHDL-2008 6.5.2 lifted the rule).
    pub(crate) fn report_out_reads(&mut self) {
        for x in std::mem::take(&mut self.out_reads) {
            let ExprKind::Obj(d) = self.exprs[x].kind else {
                continue;
            };
            let what = match &self.decls[d].kind {
                crate::decl::DeclKind::Object(o) if o.role == crate::decl::ObjRole::Port => "port",
                _ => "parameter",
            };
            let name = self.decls[d].name;
            self.report(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    format!("cannot read {what} `{name}` of mode `out`"),
                )
                .with_primary(self.exprs[x].span, "read here")
                .with_secondary(self.decls[d].span, "declared with mode `out` here")
                .with_note("VHDL-1993 does not allow the value of an `out` interface object to be read (LRM 1993 4.3.2); VHDL-2008 does")
                .with_help("declare it `inout` or `buffer`, read a separate signal or variable, or analyze the file as VHDL-2008"),
            );
        }
    }

    /// Resolve a complete context for a target of subtype `st` (assignment
    /// targets, return values, initial values): an aggregate with `others` at
    /// the top takes its bounds from `st` (9.3.3.3).
    pub(crate) fn expr_in_subtype(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        st: crate::ids::SubtypeId,
    ) -> ExprId {
        self.context_subtype = Some(st);
        let t = self.base(st);
        let id = self.expr(scope, e, Expected::Type(t));
        self.check_in_subtype(id, st);
        id
    }

    /// The actual of a generic or port (or a part of one): typed as `expr_in_subtype`, but a static
    /// scalar outside the formal's subtype is a warning, since the instance may be in a generate
    /// branch that is never elaborated; elaboration checks it (#910).
    pub(crate) fn actual_in_subtype(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        st: crate::ids::SubtypeId,
    ) -> ExprId {
        self.context_subtype = Some(st);
        let t = self.base(st);
        let id = self.expr(scope, e, Expected::Type(t));
        if !self.warn_if_outside(id, st, self.exprs[id].span) {
            self.check_in_subtype(id, st);
        }
        id
    }

    /// `W1403`: the static scalar value of `id` lies outside subtype `st`, which a qualified
    /// expression or a type conversion applies (9.3.5, 9.3.6). The value is not used and the
    /// run-time check fails (the code may never run, like `W1404`).
    pub(crate) fn warn_if_outside(
        &mut self,
        id: ExprId,
        st: crate::ids::SubtypeId,
        span: Span,
    ) -> bool {
        let (Some(v), Some((l, d, r))) = (self.value(id), self.static_bounds(st)) else {
            return false;
        };
        if crate::stat::within(v, &l, d, &r) != Some(false) {
            return false;
        }
        let range = match (l.as_int(), r.as_int()) {
            (Some(l), Some(r)) if d == crate::hir::Direction::To => format!("outside {l} to {r}"),
            (Some(l), Some(r)) => format!("outside {l} downto {r}"),
            _ => "out of range".to_string(),
        };
        let name = self.subtype_name(st);
        self.report(
            Diagnostic::warning(
                diag::W_STATIC_OUTSIDE,
                format!("static value is outside subtype `{name}`"),
            )
            .with_primary(span, range)
            .with_note(
                "the code may never run, so this is checked again at run time (LRM 9.3.5, 9.3.6)",
            ),
        );
        true
    }

    /// `E1403`: a static scalar value outside the static subtype it goes into (4.3).
    pub(crate) fn check_in_subtype(&mut self, id: ExprId, st: crate::ids::SubtypeId) {
        // 4.3: both lengths static, so the run-time length check would always fail.
        if let Some(v) = self.value(id).cloned()
            && let Some((field, want, got)) =
                self.static_mismatch(&v, self.base(st), &self.subtypes[st].constraint)
        {
            let target = match field {
                Some(f) => format!("field `{f}` of the target"),
                None => "the target".to_string(),
            };
            let span = self.exprs[id].span;
            self.report(
                Diagnostic::error(
                    diag::LENGTH_MISMATCH,
                    format!("length mismatch: {target} has {want} elements, the value {got}"),
                )
                .with_primary(span, format!("{got} elements"))
                .with_help("give the value the target's length, or change the target's constraint")
                .with_note(
                    "LRM 4.2.2.2, 9.3.6, 14.3.5: a subtype conversion, or an association with a formal, needs a matching element for each element of the target or formal",
                ),
            );
            return;
        }
        // A static composite: each scalar lies in its element or field subtype (9.3.3.3, #821).
        if let Some(v) = self.value(id).cloned()
            && let Some((x, est)) = self.elem_outside(&v, self.base(st))
        {
            let name = self.subtype_name(est);
            let span = self.exprs[id].span;
            let x = match x {
                crate::stat::ConstValue::Int(i) | crate::stat::ConstValue::Phys(i) => i.to_string(),
                crate::stat::ConstValue::Real(r) => r.to_string(),
                // An enumeration position: the literal's name.
                crate::stat::ConstValue::Enum(p) => match &self.types[self.base(est)].kind {
                    TypeKind::Enum { lits } => lits
                        .get(p as usize)
                        .map_or_else(|| p.to_string(), |&l| self.decls[l].name.to_string()),
                    _ => p.to_string(),
                },
                _ => "of the aggregate".to_string(),
            };
            self.report(
                Diagnostic::error(
                    diag::OUT_OF_SUBTYPE,
                    format!("element value {x} is outside subtype `{name}`"),
                )
                .with_primary(span, "an element out of range")
                .with_note("LRM 9.3.3.3: each element of the value belongs to the element subtype"),
            );
            return;
        }
        let (Some(v), Some((l, d, r))) = (self.value(id).cloned(), self.static_bounds(st)) else {
            return;
        };
        if crate::stat::within(&v, &l, d, &r) == Some(false) {
            let name = self.subtype_name(st);
            let span = self.exprs[id].span;
            self.report(
                Diagnostic::error(
                    diag::OUT_OF_SUBTYPE,
                    format!("static value is outside subtype `{name}`"),
                )
                .with_primary(span, "out of range"),
            );
        }
    }

    /// The first array length of the static composite `v` of type `t` that differs from the
    /// static constraint `c` or an element's or field's own: the innermost field, the
    /// constraint's length, the value's (LRM 10.6.2.1, #1005).
    fn static_mismatch(
        &self,
        v: &crate::stat::ConstValue,
        t: crate::ids::TypeId,
        c: &crate::types::Constraint,
    ) -> Option<(Option<String>, i64, i64)> {
        use crate::stat::{ConstValue as C, Elems};
        use crate::types::Constraint as K;
        match (v, &self.types[t].kind) {
            (C::Array(a), TypeKind::Array { elem, .. }) => {
                if let K::Array {
                    dims: Some(dims), ..
                } = c
                    && let Some((want, got)) =
                        dims.iter()
                            .zip(a.bounds.iter())
                            .find_map(|(rc, &(l, d, r))| {
                                let crate::types::RangeC::Explicit { left, dir, right } = rc else {
                                    return None;
                                };
                                let w = len(
                                    self.value(*left)?.as_int()?,
                                    *dir,
                                    self.value(*right)?.as_int()?,
                                );
                                let g = len(l, d, r);
                                (w != g).then_some((w, g))
                            })
                {
                    return Some((None, want, got));
                }
                let ec = match c {
                    K::Array { elem: ec, .. } if !matches!(**ec, K::None) => &**ec,
                    _ => &self.subtypes[*elem].constraint,
                };
                let Elems::Runs(runs) = &a.elems else {
                    return None;
                };
                runs.iter()
                    .find_map(|(_, x)| self.static_mismatch(x, self.base(*elem), ec))
            }
            (C::Record(fs), TypeKind::Record { elems }) => fs
                .iter()
                .zip(elems.iter())
                .enumerate()
                .find_map(|(n, (x, e))| {
                    let fc = match c {
                        K::Record { elems: cs } => {
                            cs.iter().find(|(i, _)| *i as usize == n).map(|(_, fc)| fc)
                        }
                        _ => None,
                    };
                    let fc = fc.unwrap_or(&self.subtypes[e.subtype].constraint);
                    self.static_mismatch(x, self.base(e.subtype), fc)
                        .map(|(inner, w, g)| {
                            (inner.or_else(|| Some(e.name.text.to_string())), w, g)
                        })
                }),
            _ => None,
        }
    }

    /// The first scalar of the static composite `v` of type `t` outside its element or field
    /// subtype, with that subtype.
    fn elem_outside(
        &self,
        v: &crate::stat::ConstValue,
        t: crate::ids::TypeId,
    ) -> Option<(crate::stat::ConstValue, crate::ids::SubtypeId)> {
        use crate::stat::{ConstValue as C, Elems};
        let scalar = |db: &Self, x: &C, st| {
            let (l, d, r) = db.static_bounds(st)?;
            (crate::stat::within(x, &l, d, &r) == Some(false)).then(|| (x.clone(), st))
        };
        let check = |db: &Self, x: &C, st: crate::ids::SubtypeId| match x {
            C::Array(_) | C::Record(_) => db.elem_outside(x, db.base(st)),
            _ => scalar(db, x, st),
        };
        match (v, &self.types[t].kind) {
            (C::Array(a), TypeKind::Array { elem, .. }) => match &a.elems {
                Elems::Runs(runs) => runs.iter().find_map(|(_, x)| check(self, x, *elem)),
                Elems::Bytes(b) => {
                    let mut seen = [false; 256];
                    b.iter().find_map(|&x| {
                        (!std::mem::replace(&mut seen[x as usize], true))
                            .then(|| scalar(self, &C::Enum(u32::from(x)), *elem))
                            .flatten()
                    })
                }
            },
            (C::Record(fs), TypeKind::Record { elems }) => fs
                .iter()
                .zip(elems.iter())
                .find_map(|(x, e)| check(self, x, e.subtype)),
            _ => None,
        }
    }

    /// Resolve `e` inside a complete context for a value of subtype `st` (a qualified
    /// expression's type mark, a formal): an aggregate with `others` at its top takes its
    /// bounds from `st` (9.3.3.3).
    pub(crate) fn resolve_in(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        st: crate::ids::SubtypeId,
    ) -> ExprId {
        let mut top = e;
        while let ast::ExprKind::Paren(inner) = &top.kind {
            top = inner;
        }
        let saved = self
            .top_context
            .replace((top as *const ast::Expr as usize, st));
        let t = self.base(st);
        let id = self.resolve(scope, e, Expected::Type(t));
        self.top_context = saved;
        id
    }

    /// Resolve a complete context whose value is required at analysis time
    /// (type bounds, unit scales, choices): static overflow is error `E1401`.
    pub(crate) fn static_expr(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected) -> ExprId {
        self.complete_context(scope, e, exp, true)
    }

    pub(crate) fn complete_context(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        exp: Expected,
        required: bool,
    ) -> ExprId {
        let (fresh, fresh_names) = self.memo_pool.pop().unwrap_or_default();
        let saved = std::mem::replace(&mut self.memo, fresh);
        let saved_names = std::mem::replace(&mut self.name_memo, fresh_names);
        let saved_overflow = std::mem::take(&mut self.overflow);
        let saved_top = self.top_context.take();
        let mut top = e;
        while let ast::ExprKind::Paren(inner) = &top.kind {
            top = inner;
        }
        self.top_context = self
            .context_subtype
            .take()
            .map(|st| (top as *const ast::Expr as usize, st));
        let id = self.resolve(scope, e, exp);
        self.top_context = saved_top;
        for (o, why, _) in std::mem::replace(&mut self.overflow, saved_overflow) {
            let span = self.exprs[o].span;
            let arith = why.is_none();
            let msg = match why {
                Some(w) => format!("{w} in a static expression"),
                None => format!(
                    "static arithmetic overflows `{}`",
                    self.type_name(self.base(self.exprs[o].subtype))
                ),
            };
            let d = if required {
                Diagnostic::error(diag::OVERFLOW, msg).with_primary(span, "no static value")
            } else {
                Diagnostic::warning(diag::W_OVERFLOW, msg)
                    .with_primary(span, "no static value")
                    .with_note("the value is not needed during analysis; the run-time check stays")
            };
            self.report(if arith {
                d.with_note(
                    "LRM 5.2.3.1, 5.2.4.1, 5.2.5.1: a predefined arithmetic operation delivers the correct result",
                )
            } else {
                d
            });
        }
        let mut used = std::mem::replace(&mut self.memo, saved);
        let mut used_names = std::mem::replace(&mut self.name_memo, saved_names);
        // A huge context (a big aggregate) does not leave its tables behind.
        if used.capacity() <= 112 && used_names.capacity() <= 112 {
            used.clear();
            used_names.clear();
            self.memo_pool.push((used, used_names));
        }
        id
    }

    /// Select the interpretation of `e` that fits `exp` and build its HIR.
    pub(crate) fn resolve(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected) -> ExprId {
        // The left operands of an operator chain (`a & b & c & ...`, nested to the left) are
        // resolved in this loop, not by recursion, so a long chain fits the stack.
        let mut frames = Vec::new();
        let (mut e, mut exp) = (e, exp);
        let mut id = loop {
            match self.resolve_head(scope, e, exp) {
                Head::Done(id) => break id,
                Head::Left(f) => {
                    (e, exp) = (f.left, Expected::Type(self.base(f.left_st)));
                    frames.push(f);
                }
            }
        };
        while let Some(f) = frames.pop() {
            if let Some(saved) = f.saved_top {
                self.top_context = saved;
            }
            let call = self.operator_call(scope, f.op, &f.operands, f.span, Some(id));
            id = self.implicit_conv(call, f.i, f.exp);
        }
        id
    }

    /// `resolve` up to the left operand of a binary operator, which is left to the caller's
    /// loop.
    fn resolve_head<'a>(&mut self, scope: ScopeId, e: &'a ast::Expr, exp: Expected) -> Head<'a> {
        if let ast::ExprKind::Paren(inner) = &e.kind {
            return Head::Done(self.resolve(scope, inner, exp));
        }
        // With a type from context, each alternative is checked against it: precise errors.
        if let (ast::ExprKind::Conditional(alts), Expected::Type(t)) = (&e.kind, exp) {
            return self.conditional(scope, e, alts, t).into();
        }
        let cs = self.cands(scope, e);
        match cs {
            CandSet::Class(m) => match exp {
                Expected::Type(t) if self.class_admits(m, t) => self.build_class(scope, e, t),
                Expected::Type(t) => {
                    // A string literal of a one-dimensional array type whose elements are not
                    // characters: E1330 from the literal check (15.7).
                    if let ast::ExprKind::Literal(
                        l @ ast::Literal {
                            kind: ast::LiteralKind::String(_) | ast::LiteralKind::BitString(_),
                            ..
                        },
                    ) = &e.kind
                        && matches!(&self.types[t].kind, TypeKind::Array { indexes, .. } if indexes.len() == 1)
                        && crate::expr::literal::literal_expr(self, scope, l, t).is_err()
                    {
                        return self.poison_expr().into();
                    }
                    let what = class_name(e);
                    let tn = self.type_name(t);
                    // An aggregate whose context is not composite gives it no type (9.3.3).
                    let code = match &e.kind {
                        ast::ExprKind::Aggregate(_) => diag::AGG_NO_TYPE,
                        _ => diag::TYPE_MISMATCH,
                    };
                    self.report(
                        Diagnostic::error(code, format!("expected `{tn}`, found {what}"))
                            .with_primary(e.span, format!("{what} cannot be of type `{tn}`")),
                    );
                    self.poison_expr()
                }
                _ => {
                    let (code, msg) = match &e.kind {
                        ast::ExprKind::Aggregate(_) => (
                            diag::AGG_NO_TYPE,
                            "the type of this aggregate cannot be determined from its context",
                        ),
                        _ => (
                            diag::AMBIGUOUS,
                            "the type of this literal cannot be determined from its context",
                        ),
                    };
                    self.report(
                        Diagnostic::error(code, msg)
                            .with_primary(e.span, "type not determined")
                            .with_help("qualify it with a type mark, as in `t'(...)`")
                            .with_note(
                                "LRM 9.3.2, 9.3.3.1, 12.5: the type of a string literal or an aggregate comes solely from its context, and every occurrence has exactly one interpretation",
                            ),
                    );
                    self.poison_expr()
                }
            },
            CandSet::Alloc(d) => match exp {
                // Another access type: the allocator checks its designated subtype and says so.
                Expected::Type(t) if self.alloc_admits(d, t) || self.designated(t).is_some() => {
                    self.build_class(scope, e, t)
                }
                Expected::Type(t) => {
                    let n = self.type_names(&[t, d]);
                    let (tn, dn) = (&n[0], &n[1]);
                    let diag = Diagnostic::error(
                        diag::TYPE_MISMATCH,
                        format!("expected `{tn}`, found an allocator of `{dn}`"),
                    )
                    .with_primary(
                        e.span,
                        format!("an allocator of `{dn}` cannot be of type `{tn}`"),
                    )
                    .with_note(
                        "LRM 9.3.7: an allocator returns an access type with the named designated type",
                    );
                    let diag = self.same_name_note(diag, &[t, d]);
                    self.report(diag);
                    self.poison_expr()
                }
                _ => {
                    self.report(
                        Diagnostic::error(
                            diag::AMBIGUOUS,
                            "the access type of this allocator cannot be determined from its context",
                        )
                        .with_primary(e.span, "type not determined")
                        .with_help("qualify it with a type mark, as in `t'(...)`")
                        .with_note(
                            "LRM 9.3.7: the type of the access value of an allocator is determinable solely from the context",
                        ),
                    );
                    self.poison_expr()
                }
            },
            CandSet::Types(is) => {
                if is.iter().any(|i| i.ty == self.poison.ty) {
                    return self.build_any(scope, e, is[0], exp).into();
                }
                let mut best: smallvec::SmallVec<[(Interp, u32); 4]> = smallvec::SmallVec::new();
                for i in &is {
                    if let Some(c) = self.cost(i, exp) {
                        if best.iter().any(|(b, _)| b.ty == i.ty && b.via == i.via) {
                            continue;
                        }
                        best.push((*i, c));
                    }
                }
                if exp == Expected::Condition && best.is_empty() && !is.is_empty() {
                    return self.implicit_condition(scope, e, &is).into();
                }
                let Some(min) = best.iter().map(|(_, c)| *c).min() else {
                    self.no_interp(scope, e, exp, &is);
                    return self.poison_expr().into();
                };
                best.retain(|(_, c)| *c == min);
                if best.len() > 1 && self.prefer_explicit(e, &mut best).is_err() {
                    return self.poison_expr().into();
                }
                if best.len() > 1 {
                    self.ambiguous(e, &best.iter().map(|b| b.0).collect::<Vec<_>>());
                    return self.poison_expr().into();
                }
                let i = best[0].0;
                if let ast::ExprKind::Binary { left, right, .. } = &e.kind {
                    let op = match self.operator_head(scope, i, &[left, right], e.span) {
                        Ok(op) => op,
                        Err(id) => return Head::Done(self.implicit_conv(id, i, exp)),
                    };
                    if !op.pre.contains_key(&0) {
                        let left_st = op.formals[0];
                        // A declared operator's formal is a context for the operand
                        // (`resolve_in`), until the operand is resolved.
                        let saved_top = (!op.implicit).then(|| {
                            let mut top = &**left;
                            while let ast::ExprKind::Paren(inner) = &top.kind {
                                top = inner;
                            }
                            self.top_context
                                .replace((top as *const ast::Expr as usize, left_st))
                        });
                        return Head::Left(Frame {
                            left,
                            left_st,
                            saved_top,
                            operands: [&**left, &**right],
                            op,
                            span: e.span,
                            i,
                            exp,
                        });
                    }
                    let call = self.operator_call(scope, op, &[left, right], e.span, None);
                    return Head::Done(self.implicit_conv(call, i, exp));
                }
                self.build_any(scope, e, i, exp)
            }
        }
        .into()
    }

    /// Build the chosen interpretation and insert the implicit conversion (2.8).
    fn build_any(&mut self, scope: ScopeId, e: &ast::Expr, i: Interp, exp: Expected) -> ExprId {
        let id = self.build(scope, e, i);
        self.implicit_conv(id, i, exp)
    }

    /// `id`, built as interpretation `i`, with the implicit conversion to `exp` (2.8).
    fn implicit_conv(&mut self, id: ExprId, i: Interp, exp: Expected) -> ExprId {
        let target = match exp {
            Expected::Type(t) => t,
            _ => return id,
        };
        if i.ty == target
            || i.ty == self.poison.ty
            || target == self.poison.ty
            || self.is_poison_expr(id)
        {
            return id;
        }
        let ui = self.well_known.universal_integer;
        let ur = self.well_known.universal_real;
        if Some(i.ty) != ui && Some(i.ty) != ur {
            return id;
        }
        let subtype = self.types[target].first_subtype;
        let (stat, value, span) = (
            self.exprs[id].stat,
            self.exprs[id].value,
            self.exprs[id].span,
        );
        self.add_expr(Expr {
            kind: ExprKind::ImplicitConv { operand: id },
            subtype,
            span,
            stat,
            value,
        })
    }

    fn build(&mut self, scope: ScopeId, e: &ast::Expr, i: Interp) -> ExprId {
        use ast::ExprKind as K;
        match &e.kind {
            K::Literal(l) => match crate::expr::literal::literal_expr(self, scope, l, i.ty) {
                Ok(id) => id,
                Err(_) => self.poison_expr(),
            },
            K::Paren(inner) => self.build(scope, inner, i),
            K::Name(n) => {
                let id = self.build_name(scope, n, i, e.span);
                self.named_return_context(e, id)
            }
            K::Unary { op, operand } => {
                if *op == ast::UnaryOp::Minus
                    && let K::Literal(l) = &operand.kind
                    && let Some(m) = crate::expr::literal::negated_min(self, l, i.ty)
                {
                    if self.in_std_library(scope) {
                        return crate::expr::literal::const_expr(self, e.span, i.ty, Some(m.value));
                    }
                    crate::expr::literal::negated_min_error(self, l.span, m);
                    return self.poison_expr();
                }
                let id = self.build_operator(scope, i, &[operand.as_ref()], e.span);
                if *op == ast::UnaryOp::Not {
                    self.not_stable_as_event(id)
                } else {
                    id
                }
            }
            K::Binary { left, right, .. } => {
                self.build_operator(scope, i, &[left.as_ref(), right.as_ref()], e.span)
            }
            K::Qualified { type_mark, operand } => {
                let Ok(st) = self.type_mark(scope, type_mark) else {
                    return self.poison_expr();
                };
                let op = self.resolve_in(scope, operand, st);
                let outside = self.warn_if_outside(op, st, e.span);
                let (stat, value) = (
                    self.exprs[op].stat.min(self.subtype_staticness(st)),
                    self.exprs[op].value.filter(|_| !outside),
                );
                self.add_expr(Expr {
                    kind: ExprKind::Qualified {
                        to: st,
                        operand: op,
                    },
                    subtype: st,
                    span: e.span,
                    stat,
                    value,
                })
            }
            K::Aggregate(_) => {
                self.unsupported(e.span, "aggregates");
                self.poison_expr()
            }
            K::New(_) => {
                self.unsupported(e.span, "allocators");
                self.poison_expr()
            }
            K::Conditional(alts) => self.conditional(scope, e, alts, i.ty),
            K::Unaffected | K::Error => self.poison_expr(),
        }
    }

    /// `not s'stable` (the predefined `not`, no time argument) read as `s'event` (#632): a stable
    /// time of zero is false in exactly the cycle of an event, so the value is the same, and the
    /// implicit signal the occurrence would declare (kernel work at every event of `s`) is not
    /// made. `id` is the call just built; any other expression is returned as it is.
    fn not_stable_as_event(&mut self, id: ExprId) -> ExprId {
        use crate::decl::{DeclKind, ObjRole};
        use crate::hir::{AttrPrefix, PredefAttr};
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &self.exprs[id].kind
        else {
            return id;
        };
        let [
            Arg {
                actual: Actual::Expr(a),
                ..
            },
        ] = &args[..]
        else {
            return id;
        };
        if !self.decls[*callee].implicit
            || self.decls[*callee].name.key().as_str() != "\"not\""
            || Some(self.base(self.exprs[id].subtype)) != self.well_known.boolean
        {
            return id;
        }
        let ExprKind::Obj(d) = self.exprs[*a].kind else {
            return id;
        };
        let DeclKind::Object(o) = &self.decls[d].kind else {
            return id;
        };
        let Some(def) = o.default.filter(|_| o.role == ObjRole::ImplicitSignal) else {
            return id;
        };
        let ExprKind::Attr {
            attr: PredefAttr::Stable,
            prefix: AttrPrefix::Expr(p),
            args,
        } = &self.exprs[def].kind
        else {
            return id;
        };
        if !args.is_empty() {
            return id;
        }
        let (prefix, subtype, span) = (*p, self.exprs[id].subtype, self.exprs[id].span);
        crate::attr::discard_implicit_signal(self, d);
        self.add_expr(Expr {
            kind: ExprKind::Attr {
                attr: PredefAttr::Event,
                prefix: AttrPrefix::Expr(prefix),
                args: Box::new([]),
            },
            subtype,
            span,
            stat: Staticness::None,
            value: None,
        })
    }

    fn build_operator(
        &mut self,
        scope: ScopeId,
        i: Interp,
        operands: &[&ast::Expr],
        span: Span,
    ) -> ExprId {
        match self.operator_head(scope, i, operands, span) {
            Ok(op) => self.operator_call(scope, op, operands, span, None),
            Err(id) => id,
        }
    }

    /// The callee of operator interpretation `i` and its formals; `Err` with the poison
    /// expression when it has none.
    fn operator_head(
        &mut self,
        scope: ScopeId,
        i: Interp,
        operands: &[&ast::Expr],
        span: Span,
    ) -> Result<OpHead, ExprId> {
        let Via::Call(d) = i.via else {
            return Err(self.poison_expr());
        };
        let callee = self.entity(d);
        if self.decls[callee].kind.subprogram().is_none() {
            return Err(self.poison_expr());
        }
        // A user operator with anonymous parameter types goes to its instance for the
        // operands' types (#308).
        let actuals: smallvec::SmallVec<[Option<&ast::Expr>; 2]> =
            operands.iter().map(|o| Some(*o)).collect();
        let Some((callee, pre)) = self.implicit_instance_of(scope, callee, &actuals, span, span)
        else {
            return Err(self.poison_expr());
        };
        let Some(s) = self.decls[callee].kind.subprogram() else {
            return Err(self.poison_expr());
        };
        let formals = s.params.iter().map(|p| p.subtype).collect();
        let ret = s.ret.unwrap_or(self.poison.subtype);
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        // 9.4.3: a pure function call with globally static actuals is globally static
        // (sem folds only the implicit operations, so user calls never become locally static).
        let pure = matches!(s.kind, crate::decl::SubprogramKind::Function { pure: true });
        let stat = if implicit {
            Staticness::Locally
        } else if pure {
            Staticness::Globally
        } else {
            Staticness::None
        };
        Ok(OpHead {
            callee,
            pre,
            formals,
            ret,
            implicit,
            stat,
        })
    }

    /// The call of operator `op` on `operands`; `first`, when given, is the first operand
    /// already resolved.
    fn operator_call(
        &mut self,
        scope: ScopeId,
        op: OpHead,
        operands: &[&ast::Expr],
        span: Span,
        first: Option<ExprId>,
    ) -> ExprId {
        let OpHead {
            callee,
            pre,
            formals,
            ret,
            implicit,
            mut stat,
        } = op;
        let mut hargs: smallvec::SmallVec<[Arg; 2]> = smallvec::SmallVec::new();
        for (k, (o, f)) in operands.iter().zip(formals).enumerate() {
            let id = match (pre.get(&k), first) {
                (Some(&id), _) => id,
                (None, Some(id)) if k == 0 => id,
                // The formal of a declared operator is a context, like a function's; a
                // predefined one's is the type alone (lrm-checks L170).
                (None, _) if implicit => {
                    let t = self.base(f);
                    self.resolve(scope, o, Expected::Type(t))
                }
                (None, _) => self.resolve_in(scope, o, f),
            };
            stat = stat.min(self.exprs[id].stat);
            hargs.push(Arg {
                formal: k as u16,
                actual: Actual::Expr(id),
                inertial: false,
            });
        }
        let id = self.add_expr(Expr {
            kind: ExprKind::Call {
                callee,
                args: hargs.into_vec().into(),
                method_of: None,
                implicit_cond: false,
            },
            subtype: ret,
            span,
            stat,
            value: None,
        });
        if implicit && stat == Staticness::Locally {
            self.fold_into(id);
        } else if implicit && stat == Staticness::Globally {
            self.absorb_overflows(id);
        }
        id
    }

    /// String and bit string literals, `null`, aggregates, allocators: typed by context.
    fn build_class(&mut self, scope: ScopeId, e: &ast::Expr, t: TypeId) -> ExprId {
        match &e.kind {
            ast::ExprKind::Literal(l) => {
                crate::expr::literal::literal_expr(self, scope, l, t).unwrap_or(self.poison.expr)
            }
            ast::ExprKind::Aggregate(assocs) => {
                // The context subtype, when the caller gave one (an object's declared subtype).
                let ctx = match self.top_context {
                    Some((node, st))
                        if node == e as *const ast::Expr as usize && self.base(st) == t =>
                    {
                        st
                    }
                    // A constrained array type is no context: an operand, say, takes the
                    // direction of the index subtype (lrm-checks L170, #1009).
                    _ => self.types[t].base_subtype,
                };
                let mut r = Resolver { scope };
                crate::expr::aggregate::resolve_aggregate(self, &mut r, assocs, ctx, e.span)
                    .unwrap_or(self.poison.expr)
            }
            ast::ExprKind::New(a) => self.allocator(scope, a, t, e.span),
            ast::ExprKind::Conditional(alts) => self.conditional(scope, e, alts, t),
            _ => {
                self.ice(
                    e.span,
                    "a class-typed expression that is not a literal, aggregate or allocator",
                );
                self.poison_expr()
            }
        }
    }

    /// A call of a function with a named return value (4.2.1, 2019) gets the context
    /// subtype as its result subtype: M2 passes its descriptor to the function. Without a
    /// fully constrained context subtype it is `E1304` (lrm-checks L62).
    fn named_return_context(&mut self, e: &ast::Expr, id: ExprId) -> ExprId {
        let ExprKind::Call { callee, .. } = self.exprs[id].kind else {
            return id;
        };
        let Some(ps) = self.decls[callee].kind.subprogram().and_then(|s| s.scope) else {
            return id;
        };
        if !self.ret_names.contains_key(&ps) {
            return id;
        }
        let node = e as *const ast::Expr as usize;
        if self.return_node == Some(node) {
            let name = self.decls[callee].name;
            self.report(
                Diagnostic::error(
                    diag::NAMED_RETURN_CONTEXT,
                    format!(
                        "`{name}` has a named return value and cannot be the value of a `return` \
                         statement: a return statement gives it no result subtype \
                         (LCS-2016-072b, VHDL-2019 4.2.1)"
                    ),
                )
                .with_primary(e.span, "called in a return statement")
                .with_help(
                    "assign the call to a variable of the result subtype and return that \
                     variable, or qualify the call with a constrained subtype (`t'(f)`)",
                ),
            );
            return self.poison_expr();
        }
        match self.top_context {
            Some((node, st))
                if node == e as *const ast::Expr as usize
                    && self.base(st) == self.base(self.exprs[id].subtype)
                    && self.constrainedness(st) == crate::types::Constrainedness::Full =>
            {
                self.exprs[id].subtype = st;
                id
            }
            _ => {
                let name = self.decls[callee].name;
                self.report(
                    Diagnostic::error(
                        diag::NAMED_RETURN_CONTEXT,
                        format!(
                            "`{name}` has a named return value, but this context gives no \
                             constrained result subtype"
                        ),
                    )
                    .with_primary(e.span, "result subtype not known here")
                    .with_help(
                        "assign the call to a constrained object, or qualify it with a \
                         constrained subtype",
                    ),
                );
                self.poison_expr()
            }
        }
    }

    /// A 2019 conditional expression `a when c else b` of type `t` (9.3.x): every
    /// alternative is of type `t` and takes the context subtype, if any; static when
    /// all its parts are (permissive), folded when the chosen alternative is known.
    fn conditional(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        alts: &[(ast::Expr, Option<ast::Expr>)],
        t: TypeId,
    ) -> ExprId {
        let ctx = match self.top_context {
            Some((node, st)) if node == e as *const ast::Expr as usize && self.base(st) == t => {
                Some(st)
            }
            _ => None,
        };
        let mut out = Vec::with_capacity(alts.len());
        let mut stat = Staticness::Locally;
        // Folding: the value of the first alternative whose condition is statically true.
        let (mut value, mut open) = (None, true);
        for (v, c) in alts {
            let v = match ctx {
                Some(st) => self.resolve_in(scope, v, st),
                None => self.resolve(scope, v, Expected::Type(t)),
            };
            let c = c
                .as_ref()
                .map(|c| self.resolve(scope, c, Expected::Condition));
            stat = stat.min(self.exprs[v].stat);
            if let Some(c) = c {
                stat = stat.min(self.exprs[c].stat);
            }
            if open {
                match c.map(|c| self.value(c)) {
                    None | Some(Some(crate::stat::ConstValue::Enum(1))) => {
                        value = self.exprs[v].value;
                        open = false;
                    }
                    Some(Some(crate::stat::ConstValue::Enum(0))) => {}
                    Some(_) => open = false,
                }
            }
            out.push((v, c));
        }
        if out
            .iter()
            .any(|&(v, c)| self.is_poison_expr(v) || c.is_some_and(|c| self.is_poison_expr(c)))
        {
            return self.poison_expr();
        }
        let subtype = ctx.unwrap_or(self.types[t].first_subtype);
        self.add_expr(Expr {
            kind: ExprKind::Conditional { alts: out.into() },
            subtype,
            span: e.span,
            stat,
            value,
        })
    }

    /// 9.2.9: a condition of a type other than BOOLEAN goes through a visible `??`.
    /// A boolean of a PSL directive in a 1993 file, of type `bit` or `std_ulogic`: 1993 has no
    /// `??`, and PSL takes these types as booleans in any revision, so it is `e = '1'` (#911).
    // ponytail: `'H'` is false here and true under the 2008 `??`; needs the `??` table if a 1993
    // design relies on it.
    fn psl_bool_93(&mut self, scope: ScopeId, e: &ast::Expr, is: &[Interp]) -> Option<ExprId> {
        if self.file_rev() != crate::LangRev::Vhdl1993
            || !matches!(
                self.stmt_owners.last(),
                Some(crate::stmt::Owner::Process { psl: true, .. })
            )
        {
            return None;
        }
        let t = is.iter().map(|i| i.ty).find(|&t| {
            let n = self.type_name(t).to_ascii_uppercase();
            n == "BIT" || n == "STD_ULOGIC"
        })?;
        let boolean = self.well_known.boolean?;
        let sc = self.decls[self.types[t].decl].scope;
        let eq = self.scopes[sc]
            .names
            .get(&operator_key("="))?
            .iter()
            .copied()
            .find(|&d| {
                self.decls[d].kind.subprogram().is_some_and(|s| {
                    s.params.len() == 2
                        && s.params.iter().all(|p| self.base(p.subtype) == t)
                        && s.ret.map(|r| self.base(r)) == Some(boolean)
                })
            })?;
        let one = ast::Expr {
            kind: ast::ExprKind::Literal(ast::Literal {
                kind: ast::LiteralKind::Char('1'),
                span: e.span,
            }),
            span: e.span,
        };
        let (op, one) = (
            self.resolve(scope, e, Expected::Type(t)),
            self.resolve(scope, &one, Expected::Type(t)),
        );
        let arg = |formal, x| Arg {
            formal,
            actual: Actual::Expr(x),
            inertial: false,
        };
        Some(self.add_expr(Expr {
            kind: ExprKind::Call {
                callee: eq,
                args: Box::new([arg(0, op), arg(1, one)]),
                method_of: None,
                implicit_cond: false,
            },
            subtype: self.types[boolean].first_subtype,
            span: e.span,
            stat: self.exprs[op].stat.min(Staticness::Globally),
            value: None,
        }))
    }

    fn implicit_condition(&mut self, scope: ScopeId, e: &ast::Expr, is: &[Interp]) -> ExprId {
        let boolean = self.well_known.boolean;
        let mut found: Vec<(DeclId, TypeId)> = Vec::new();
        let key = operator_key("??");
        let visible = match self.lookup(scope, key) {
            Visible::Overloads(ds) => ds.to_vec(),
            _ => Vec::new(),
        };
        // 9.2.9 does not say where the `??` comes from. Permissive (L53, user 2026-10-19): when
        // none is visible, take the one declared next to the condition's type, so
        // `use ieee.std_logic_1164.std_ulogic` alone is enough.
        let beside: Vec<DeclId> = is
            .iter()
            .flat_map(|i| {
                let sc = self.decls[self.types[i.ty].decl].scope;
                self.scopes[sc].names.get(&key).cloned().unwrap_or_default()
            })
            .collect();
        for pass in [visible, beside] {
            if !found.is_empty() {
                break;
            }
            for d in pass {
                let Some(s) = self.decls[self.entity(d)].kind.subprogram() else {
                    continue;
                };
                if s.params.len() != 1 || s.ret.map(|r| self.base(r)) != boolean {
                    continue;
                }
                let pt = self.base(s.params[0].subtype);
                if is
                    .iter()
                    .any(|i| self.cost(i, Expected::Type(pt)).is_some())
                    && !found.iter().any(|(f, _)| self.entity(*f) == self.entity(d))
                {
                    found.push((d, pt));
                }
            }
        }
        match found.as_slice() {
            [(d, pt)] => {
                let callee = self.entity(*d);
                let op = self.resolve(scope, e, Expected::Type(*pt));
                let ret = self
                    .well_known
                    .boolean
                    .map_or(self.poison.subtype, |b| self.types[b].first_subtype);
                let stat = self.exprs[op].stat.min(Staticness::Globally);
                self.add_expr(Expr {
                    kind: ExprKind::Call {
                        callee,
                        args: Box::new([Arg {
                            formal: 0,
                            actual: Actual::Expr(op),
                            inertial: false,
                        }]),
                        method_of: None,
                        implicit_cond: true,
                    },
                    subtype: ret,
                    span: e.span,
                    stat,
                    value: None,
                })
            }
            [] if let Some(x) = self.psl_bool_93(scope, e, is) => x,
            [] => {
                let found = is.first().map(|i| self.type_name(i.ty));
                let mut d = Diagnostic::error(diag::NOT_BOOLEAN, "condition is not BOOLEAN")
                    .with_primary(
                        e.span,
                        match &found {
                            Some(t) => format!("this is of type `{t}`"),
                            None => "not a BOOLEAN expression".into(),
                        },
                    )
                    .with_note(
                        "LRM 9.2.9: a condition is of type BOOLEAN, or the condition operator is implicitly applied to it",
                    );
                if let Some(t) = found {
                    d = d.with_help(format!(
                        "no `??` is visible for `{t}`; is `ieee.std_logic_1164` used?"
                    ));
                }
                self.report(d);
                self.poison_expr()
            }
            _ => {
                let mut d = Diagnostic::error(
                    diag::AMBIGUOUS_CONDITION,
                    "several `??` operators apply to this condition",
                )
                .with_primary(e.span, "ambiguous condition")
                .with_note(
                    "LRM 9.2.9, 12.5: the operator implicitly applied to a condition is determined by overload resolution",
                );
                let ops: Vec<DeclId> = found.iter().map(|f| f.0).collect();
                for (&op, sig) in ops.iter().zip(self.signatures(&ops, &[])) {
                    d = d.with_secondary(self.decls[op].span, sig);
                }
                self.report(d);
                self.poison_expr()
            }
        }
    }

    /// Report why `e` has no interpretation that fits `exp`.
    fn no_interp(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected, is: &[Interp]) {
        use ast::ExprKind as K;
        if self.anonymous_operator_use(scope, e) {
            return;
        }
        // A character literal that is not a literal of the expected enumeration type (15.6).
        if let (
            K::Literal(ast::Literal {
                kind: ast::LiteralKind::Char(c),
                ..
            }),
            Expected::Type(t),
        ) = (&e.kind, exp)
            && matches!(self.types[t].kind, TypeKind::Enum { .. })
            && crate::expr::literal::char_literal(self, *c, t, e.span).is_err()
        {
            return;
        }
        // Names: missing declarations and unsupported forms first.
        if let K::Name(n) = &e.kind {
            let base = match &n.kind {
                ast::NameKind::Apply { prefix, .. } => prefix.as_ref(),
                _ => n,
            };
            let base = match &base.kind {
                ast::NameKind::Attribute { prefix, .. } => prefix.as_ref(),
                _ => base,
            };
            let r = self.classify(scope, base);
            if matches!(
                r,
                NameRes::Missing(_)
                    | NameRes::Conflict(..)
                    | NameRes::Unsupported(..)
                    | NameRes::NotInRegion(..)
                    | NameRes::Uninstantiated(..)
                    | NameRes::Error
            ) {
                self.report_name(scope, r, base.span, "a name");
                return;
            }
            if let Some(g) = self.uninstantiated_of(&r) {
                self.uninstantiated(g, base.span);
                return;
            }
            // `T'a.f` and `T'a(i).f`: the real problem is the attribute under the selections.
            let mut under = n;
            while let ast::NameKind::Selected { prefix, .. } | ast::NameKind::Apply { prefix, .. } =
                &under.kind
            {
                under = prefix;
            }
            if let ast::NameKind::Attribute { prefix, attr } = &under.kind
                && !std::ptr::eq(under, n)
                && self.report_attr(scope, prefix, *attr)
            {
                return;
            }
            let attr = match &n.kind {
                ast::NameKind::Attribute { attr, .. } => Some(*attr),
                ast::NameKind::Apply { prefix, .. } => match &prefix.kind {
                    ast::NameKind::Attribute { attr, .. } => Some(*attr),
                    _ => None,
                },
                _ => None,
            };
            if let Some(attr) = attr {
                if self.report_attr(scope, base, attr) {
                    return;
                }
                // An applied attribute whose value is indexed or sliced: the
                // generic apply diagnostics below report a wrong arity or index
                // type; only a bare attribute name is unsupported here (#653).
                // With interpretations, the type mismatch below is the problem (#822).
                if !matches!(n.kind, ast::NameKind::Apply { .. }) && is.is_empty() {
                    self.unsupported(n.span, "this use of a predefined attribute");
                    return;
                }
            }
            // `obj.m` of a protected object without a method `m` (E1100, E1105).
            if let ast::NameKind::Selected {
                prefix: obj,
                suffix: ast::Suffix::Ident(m),
            } = &base.kind
                && let Ok(ts) = self.prefix_types(scope, obj)
                && let Some((pt, _)) = ts.iter().find_map(|&t| self.method_target(t))
                && self.protected_member(pt, *m, scope).is_err()
            {
                return;
            }
        }
        if is.is_empty() {
            // An operand without any interpretation is the real problem (cascade, 8.2).
            let operands: Vec<&ast::Expr> = match &e.kind {
                K::Unary { operand, .. } => vec![operand],
                K::Binary { left, right, .. } => vec![left, right],
                _ => vec![],
            };
            for &o in &operands {
                if matches!(self.cands(scope, o), CandSet::Types(v) if v.is_empty()) {
                    self.resolve(scope, o, Expected::Any);
                    return;
                }
            }
            let (what, sym) = match &e.kind {
                K::Unary { op, .. } => ("operator", Some(unary_symbol(*op))),
                K::Binary { op, .. } => ("operator", Some(binary_symbol(*op))),
                _ => ("name", None),
            };
            if let Some(sym) = sym {
                // A protected type has no predefined operators (5.6.1): say so (E1905).
                let prot = operands.iter().find_map(|&o| match self.cands(scope, o) {
                    CandSet::Types(v) => v
                        .iter()
                        .find(|i| self.type_contains_protected(i.ty))
                        .map(|i| i.ty),
                    CandSet::Class(_) | CandSet::Alloc(_) => None,
                });
                if let Some(t) = prot {
                    self.report(
                        Diagnostic::error(
                            diag::PROTECTED_MISUSE,
                            format!(
                                "operator `{sym}` is not defined for {}",
                                self.protected_desc(t)
                            ),
                        )
                        .with_primary(e.span, "operand of a protected type")
                        .with_note("LRM 5.6.1: a protected type has no predefined operators"),
                    );
                    return;
                }
                let mut d = Diagnostic::error(
                    diag::NO_OVERLOAD,
                    format!("no visible `\"{sym}\"` accepts these operands"),
                )
                .with_primary(e.span, format!("no matching {what}"))
                .with_note(
                    "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
                );
                if let Some(h) = self.vector_integer_hint(scope, e) {
                    d = d.with_help(h);
                }
                if let Visible::Overloads(ds) = self.lookup(scope, operator_key(sym)) {
                    let n = ds.len();
                    d = self.candidate_labels(d, &ds[..n.min(MAX_CANDIDATE_LABELS)], &[]);
                    if n > MAX_CANDIDATE_LABELS {
                        d = d.with_note(format!("and {} more", n - MAX_CANDIDATE_LABELS));
                    }
                }
                self.report(d);
                return;
            }
            if let K::Name(ast::Name {
                kind: ast::NameKind::Apply { prefix, args },
                ..
            }) = &e.kind
            {
                // A prefix that is only values (no function): say why it cannot be indexed.
                if let CandSet::Types(ps) = self.name_cands(scope, prefix)
                    && !ps.is_empty()
                    && ps.iter().all(|i| !matches!(i.via, Via::Call(_)))
                    && let Some(first) = args.first()
                {
                    let pname = self.source_text(prefix.span);
                    let arrays: Vec<usize> = ps
                        .iter()
                        .filter_map(|i| match &self.types[i.ty].kind {
                            TypeKind::Array { indexes, .. } => Some(indexes.len()),
                            _ => None,
                        })
                        .collect();
                    if arrays.is_empty()
                        && ps.iter().all(|i| {
                            !matches!(
                                self.types[i.ty].kind,
                                TypeKind::Access { .. } | TypeKind::Error
                            )
                        })
                    {
                        let tn = self.type_name(ps[0].ty);
                        self.report(
                            Diagnostic::error(
                                diag::TYPE_MISMATCH,
                                format!("`{pname}` has type `{tn}` and cannot be indexed"),
                            )
                            .with_primary(first.span, "index of a value that is not an array")
                            .with_secondary(prefix.span, format!("of type `{tn}`"))
                            .with_note(
                                "LRM 8.4: the prefix of an indexed name is appropriate for an array type",
                            ),
                        );
                        return;
                    }
                    if let [dims] = arrays[..]
                        && dims != args.len()
                    {
                        self.report(
                            Diagnostic::error(
                                diag::INDEX_ARITY,
                                format!(
                                    "array `{pname}` has {dims} dimension{}, found {} indexes",
                                    if dims == 1 { "" } else { "s" },
                                    args.len()
                                ),
                            )
                            .with_primary(first.span, format!("{} indexes", args.len()))
                            .with_note(
                                "LRM 8.4: an indexed name has one expression for each index position of the array",
                            ),
                        );
                        return;
                    }
                }
                if let NameRes::Overloads(ds) = self.classify(scope, prefix) {
                    self.no_function(scope, prefix, args, &ds);
                    return;
                }
                self.report(
                    Diagnostic::error(
                        diag::NO_OVERLOAD,
                        "no visible subprogram accepts these arguments",
                    )
                    .with_primary(prefix.span, "no matching subprogram")
                    .with_note(
                        "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
                    ),
                );
                return;
            }
            if let K::Name(n) = &e.kind
                && let NameRes::Overloads(ds) = self.classify(scope, n)
                && self.procedures_only(n, &ds)
            {
                return;
            }
            // `f(x).m`: a call under the selections that no overload accepts is the real
            // problem, reported like a bare call (type mismatch of the argument).
            if let K::Name(n) = &e.kind {
                let mut under = n;
                while let ast::NameKind::Selected { prefix, .. } = &under.kind {
                    under = prefix;
                }
                if let ast::NameKind::Apply { prefix, args } = &under.kind
                    && !std::ptr::eq(under, n)
                    && matches!(self.name_cands(scope, under), CandSet::Types(v) if v.is_empty())
                {
                    if let NameRes::Overloads(ds) = self.classify(scope, prefix) {
                        self.no_function(scope, prefix, args, &ds);
                    } else {
                        self.report(
                            Diagnostic::error(
                                diag::NO_OVERLOAD,
                                "no visible subprogram accepts these arguments",
                            )
                            .with_primary(prefix.span, "no matching subprogram")
                            .with_note(
                                "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
                            ),
                        );
                    }
                    return;
                }
            }
            if let K::Name(n) = &e.kind
                && self.unknown_element(scope, n)
            {
                return;
            }
            self.report(
                Diagnostic::error(diag::NOT_AN_OBJECT, "this name does not denote a value")
                    .with_primary(e.span, "not an object, literal or function"),
            );
            return;
        }
        // Only function calls, none returning the expected type (12.5).
        if let (K::Name(_), Expected::Type(t)) = (&e.kind, exp)
            && let Some(calls) = is
                .iter()
                .map(|i| match i.via {
                    Via::Call(d) => Some(self.entity(d)),
                    _ => None,
                })
                .collect::<Option<Vec<DeclId>>>()
        {
            let ts: Vec<TypeId> = std::iter::once(t).chain(is.iter().map(|i| i.ty)).collect();
            let tn = self.type_names(&ts).swap_remove(0);
            let mut d = Diagnostic::error(
                diag::NO_OVERLOAD,
                format!("no function `{}` returns `{tn}`", self.decls[calls[0]].name),
            )
            .with_primary(e.span, format!("expected `{tn}`"))
            .with_note(
                "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
            );
            d = self.candidate_labels(d, &calls[..calls.len().min(MAX_CANDIDATE_LABELS)], &[t]);
            d = self.same_name_note(d, &ts);
            self.report(d);
            return;
        }
        let mut ts: Vec<TypeId> = is.iter().map(|i| i.ty).collect();
        if let Expected::Type(t) = exp {
            ts.push(t);
        }
        let mut names = self.type_names(&ts);
        let want_name = matches!(exp, Expected::Type(_))
            .then(|| names.pop())
            .flatten();
        let found: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
        let want = match exp {
            Expected::Type(_) => format!("`{}`", want_name.unwrap_or_default()),
            Expected::Condition => "BOOLEAN".into(),
            Expected::Discrete => "a discrete type".into(),
            Expected::Composite => "a composite type".into(),
            Expected::Any => "a value".into(),
        };
        let mut d = Diagnostic::error(
            diag::TYPE_MISMATCH,
            format!("expected {want}, found {}", found.join(" or ")),
        )
        .with_primary(e.span, format!("expected {want}"))
        .with_note(
            "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context, given the type it requires",
        );
        if let ([i], Expected::Type(t)) = (is, exp)
            && let Some(h) = self.conversion_hint(i.ty, t)
        {
            d = d.with_help(h);
        }
        d = self.same_name_note(d, &ts);
        self.report(d);
    }

    /// Two types of `ts` that still print the same in `type_names`, where a path cannot tell them
    /// apart: say where each is declared.
    pub fn same_name_note(&self, d: Diagnostic, ts: &[TypeId]) -> Diagnostic {
        let names = self.type_names(ts);
        let same = |i: usize, j: usize| ts[i] != ts[j] && names[i].eq_ignore_ascii_case(&names[j]);
        let Some((i, j)) = (0..ts.len())
            .flat_map(|i| (i + 1..ts.len()).map(move |j| (i, j)))
            .find(|&(i, j)| same(i, j))
        else {
            return d;
        };
        let name = &names[i];
        d.with_secondary(
            self.decls[self.types[ts[i]].decl].span,
            format!("one `{name}` is declared here"),
        )
        .with_secondary(
            self.decls[self.types[ts[j]].decl].span,
            format!("another `{name}` is declared here"),
        )
        .with_note(format!(
            "these are two different types named `{name}`: they differ in the declaration or the analysis of its design unit they come from"
        ))
    }

    /// 8.3: how to turn a value of type `from` into the expected type `to`: a type
    /// conversion between closely related types (9.3.6), `to_unsigned`/`to_signed` from an
    /// integer.
    fn conversion_hint(&self, from: TypeId, to: TypeId) -> Option<String> {
        // ponytail: keyed on the numeric_std/numeric_bit names like `vector_integer_hint`;
        // `UNRESOLVED_UNSIGNED` is shown as the subtype `unsigned` users write.
        let name = self.type_names(&[from, to]).swap_remove(1);
        let lower = name.to_ascii_lowercase();
        let mark = match lower.strip_prefix("unresolved_") {
            Some(m) => m.to_string(),
            None if name == name.to_ascii_uppercase() => lower.clone(),
            None => name,
        };
        let integer = matches!(
            self.types[from].kind,
            TypeKind::Integer | TypeKind::UniversalInteger
        );
        if integer && matches!(mark.as_str(), "unsigned" | "signed") {
            return Some(format!("use `to_{mark}(x, n)` to convert"));
        }
        let composite = |t: TypeId| matches!(self.types[t].kind, TypeKind::Array { .. });
        (composite(from) && self.closely_related(from, to))
            .then(|| format!("use `{mark}(x)` to convert"))
    }

    /// Calls that are ambiguous only between implicit operations and explicit
    /// subprograms (a 2019 composite `TO_STRING` and a user `to_string(A; sep := " ")`,
    /// in the same package or made visible by a use clause): the explicit ones win
    /// (decisions.md). In a 1993 file an implicit and an explicit
    /// homograph made visible by use clauses stay ambiguous (1993 10.4, 10.5): `E1302`, or
    /// `W1302` and the explicit one under `--relaxed` (R7, lrm-checks L134).
    fn prefer_explicit(
        &mut self,
        e: &ast::Expr,
        best: &mut smallvec::SmallVec<[(Interp, u32); 4]>,
    ) -> Result<(), crate::Reported> {
        let calls: Option<Vec<DeclId>> = best
            .iter()
            .map(|(i, _)| match i.via {
                Via::Call(d) => Some(self.entity(d)),
                _ => None,
            })
            .collect();
        let Some(calls) = calls else { return Ok(()) };
        let (imp, exp): (Vec<DeclId>, Vec<DeclId>) =
            calls.iter().partition(|&&d| self.decls[d].implicit);
        if imp.is_empty() || exp.is_empty() {
            return Ok(());
        }
        if self.file_rev() == crate::LangRev::Vhdl1993
            && let Some((&i, &x)) = imp.iter().find_map(|i| {
                exp.iter()
                    .find(|&&x| self.homographs(*i, x))
                    .map(|x| (i, x))
            })
        {
            let name = self.decls[x].name;
            let [ri, rx] = [i, x].map(|d| crate::mangle::region(self, self.decls[d].scope));
            let what = format!(
                "`{name}` is declared implicitly by `{ri}` and explicitly by `{rx}`; VHDL-1993 \
                 hides neither (LRM 1993 10.4, 10.5)"
            );
            if self.relaxed() {
                self.report(
                    Diagnostic::warning(
                        diag::W_IMPLICIT_HOMOGRAPH_93,
                        format!("{what}; `--relaxed` uses the explicit one"),
                    )
                    .with_primary(e.span, "accepted by `--relaxed`"),
                );
            } else {
                let interps: Vec<Interp> = best.iter().map(|b| b.0).collect();
                self.ambiguous_with(
                    e,
                    &interps,
                    Some((
                        what,
                        format!(
                            "analyse the file with `--std=2008`, where the explicit one wins \
                             (LRM 2008 12.4), pass `--relaxed`, or replace `use {rx}.all` by \
                             use clauses for only the operations you need from it, without \
                             {name}"
                        ),
                    )),
                );
                return Err(crate::Reported);
            }
        }
        best.retain(|(i, _)| matches!(i.via, Via::Call(d) if !self.decls[self.entity(d)].implicit));
        Ok(())
    }

    fn ambiguous(&mut self, e: &ast::Expr, best: &[Interp]) {
        self.ambiguous_with(e, best, None);
    }

    /// `ambiguous` with a message and help of the caller's (`why`, `help`).
    fn ambiguous_with(&mut self, e: &ast::Expr, best: &[Interp], why: Option<(String, String)>) {
        let msg = why
            .as_ref()
            .map_or("this expression is ambiguous".into(), |w| w.0.clone());
        let mut d = Diagnostic::error(diag::AMBIGUOUS, msg)
            .with_primary(e.span, "several interpretations fit")
            .with_note(
                "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
            );
        let callables: Vec<DeclId> = best
            .iter()
            .filter_map(|i| match i.via {
                Via::Call(c) | Via::Method(c, _) | Via::EnumLit(c) => Some(c),
                _ => None,
            })
            .collect();
        let others: Vec<TypeId> = best
            .iter()
            .filter(|i| !matches!(i.via, Via::Call(_) | Via::Method(..) | Via::EnumLit(_)))
            .map(|i| i.ty)
            .collect();
        let mut sigs = self.signatures(&callables, &others).into_iter();
        let mut names = self.type_names(&others).into_iter();
        for i in best {
            match i.via {
                Via::Call(c) | Via::Method(c, _) | Via::EnumLit(c) => {
                    let sig = sigs.next().expect("one per callable");
                    let sp = self.decls[c].span;
                    d = if sp != Span::default() {
                        d.with_secondary(sp, sig)
                    } else {
                        d.with_note(format!("candidate: {sig}"))
                    };
                }
                _ => {
                    let n = names.next().expect("one per other");
                    d = d.with_note(format!("could be of type `{n}`"));
                }
            }
        }
        // A call whose candidates come from several packages: name the first one.
        let callee = match &e.kind {
            ast::ExprKind::Name(ast::Name {
                kind: ast::NameKind::Apply { prefix, .. },
                ..
            }) => Some(prefix.span),
            ast::ExprKind::Name(n) => Some(n.span),
            _ => None,
        };
        let regions: Option<Vec<String>> = best
            .iter()
            .map(|i| match i.via {
                Via::Call(c) => Some(crate::mangle::region(
                    self,
                    self.decls[self.entity(c)].scope,
                )),
                _ => None,
            })
            .collect();
        let help = match (callee, regions) {
            (Some(sp), Some(rs)) if rs.iter().any(|r| *r != rs[0]) && !rs[0].is_empty() => {
                format!(
                    "qualify the operand or use a selected name (`{}.{}`)",
                    rs[0],
                    self.source_text(sp)
                )
            }
            _ => "qualify an operand (`t'(x)`) or use a selected name (`pkg.\"+\"`)".into(),
        };
        let help = why.map_or(help, |w| w.1);
        self.report(d.with_help(help));
    }

    /// The name of a type for messages (8.3): anonymous base types say so.
    pub fn type_name(&self, t: TypeId) -> String {
        let td = &self.types[t];
        match &td.kind {
            TypeKind::UniversalInteger => "universal_integer".into(),
            TypeKind::UniversalReal => "universal_real".into(),
            _ => td.name.text.to_string(),
        }
    }

    /// Names of `ts` for one message: the short names, but two different types that read alike
    /// both print their full path, `work.pkg_a.rc` and `work.pkg_b.rc` (#1026).
    /// [`Db::same_name_note`] covers two paths that still read alike.
    pub fn type_names(&self, ts: &[TypeId]) -> Vec<String> {
        let short = ts.iter().map(|&t| self.type_name(t)).collect();
        names_apart(ts, short, |i| self.type_path(ts[i]))
    }

    /// [`Db::type_names`] for subtypes: the name each has in a message, compared by what it
    /// names. Two anonymous subtypes of one type are one name, however they are constrained.
    pub fn subtype_names(&self, sts: &[crate::ids::SubtypeId]) -> Vec<String> {
        let keys: Vec<_> = sts.iter().map(|&st| self.subtype_key(st)).collect();
        let short = sts.iter().map(|&st| self.subtype_name(st)).collect();
        names_apart(&keys, short, |i| match keys[i].1 {
            Some(st) => self
                .decls
                .iter()
                .find(|(_, d)| matches!(d.kind, crate::decl::DeclKind::Subtype(s) if s == st))
                .map_or_else(|| self.subtype_name(st), |(d, _)| self.decl_path(d)),
            None => self.type_path(keys[i].0),
        })
    }

    /// [`Db::type_names`] for declarations of other kinds: subprograms, components, packages.
    pub fn decl_names(&self, ds: &[crate::ids::DeclId]) -> Vec<String> {
        let short = ds.iter().map(|&d| self.decls[d].name.to_string()).collect();
        names_apart(ds, short, |i| self.decl_path(ds[i]))
    }

    /// What a subtype's printed name stands for: its base type, and the subtype itself if it has
    /// a name of its own.
    fn subtype_key(&self, st: crate::ids::SubtypeId) -> (TypeId, Option<crate::ids::SubtypeId>) {
        let base = self.base(st);
        let own = self.subtypes[st]
            .name
            .is_some_and(|n| n.key != self.types[base].name.key);
        (base, own.then_some(st))
    }

    /// `work.pkg.rc`: a type by the regions around its declaration; universal types have none.
    fn type_path(&self, t: TypeId) -> String {
        let td = &self.types[t];
        match td.kind {
            TypeKind::UniversalInteger | TypeKind::UniversalReal => self.type_name(t),
            _ => join_path(
                self.region_path(self.decls[td.decl].scope),
                &self.type_name(t),
            ),
        }
    }

    fn decl_path(&self, d: crate::ids::DeclId) -> String {
        let decl = &self.decls[d];
        join_path(self.region_path(decl.scope), &decl.name.to_string())
    }

    /// The region of `scope` as a message writes it: `work.pkg` for a package and its body,
    /// `work.top(rtl)` for an architecture, then the plain name of each nested region that has
    /// one. Not [`crate::mangle::region`]: that one spells out subprogram profiles.
    fn region_path(&self, scope: ScopeId) -> String {
        let s = &self.scopes[scope];
        let Some(owner) = s.owner else {
            return s.parent.map_or_else(String::new, |p| self.region_path(p));
        };
        let d = &self.decls[owner];
        let crate::decl::DeclKind::Unit(u) = &d.kind else {
            return join_path(self.region_path(d.scope), &d.name.to_string());
        };
        if let Some(m) = self.generic.unit_names.get(u) {
            return m.as_str().to_string();
        }
        let u = &self.units[*u];
        let lib = self.libs[u.lib].name.key.as_str();
        match u.key {
            crate::loader::UnitKey::Primary(p) => format!("{lib}.{p}", p = p.as_str()),
            crate::loader::UnitKey::Secondary(p, b) if b.as_str() == "body" => {
                format!("{lib}.{p}", p = p.as_str())
            }
            crate::loader::UnitKey::Secondary(p, a) => {
                format!("{lib}.{p}({a})", p = p.as_str(), a = a.as_str())
            }
        }
    }

    /// `function "+" (l: unsigned; r: natural) return unsigned` (8.3), original spelling.
    pub fn signature(&self, d: crate::ids::DeclId) -> String {
        self.signatures(&[d], &[]).remove(0)
    }

    /// [`Db::signature`] of each of `ds` for one message that also names the types `others`:
    /// marks that read alike but mean different types print their paths, and subprograms whose
    /// whole text still reads alike print their own path (#1026).
    pub(crate) fn signatures(&self, ds: &[crate::ids::DeclId], others: &[TypeId]) -> Vec<String> {
        let mut marks: Vec<crate::ids::SubtypeId> = Vec::new();
        for &d in ds {
            let e = self.entity(d);
            match &self.decls[e].kind {
                crate::decl::DeclKind::EnumLit { ty, .. } => {
                    marks.push(self.types[*ty].first_subtype);
                }
                k => {
                    if let Some(s) = k.subprogram() {
                        marks.extend(s.params.iter().map(|p| p.subtype));
                        marks.extend(s.ret);
                    }
                }
            }
        }
        marks.extend(others.iter().map(|&t| self.types[t].first_subtype));
        let names = self.subtype_names(&marks);
        let name_of = |st| {
            let i = marks
                .iter()
                .position(|&m| m == st)
                .expect("collected above");
            names[i].as_str()
        };
        let text = |d: crate::ids::DeclId, path: bool| {
            let e = self.entity(d);
            let decl = &self.decls[e];
            let name = if path {
                self.decl_path(e)
            } else {
                decl.name.to_string()
            };
            match decl.kind.subprogram() {
                Some(s) => {
                    let params: Vec<String> = s
                        .params
                        .iter()
                        .map(|p| format!("{}: {}", p.name.text, name_of(p.subtype)))
                        .collect();
                    let head = match s.kind {
                        crate::decl::SubprogramKind::Procedure => "procedure",
                        crate::decl::SubprogramKind::Function { .. } => "function",
                    };
                    let mut out = format!("{head} {name}");
                    if !params.is_empty() {
                        out += &format!(" ({})", params.join("; "));
                    }
                    if let Some(r) = s.ret {
                        out += &format!(" return {}", name_of(r));
                    }
                    out
                }
                None => match decl.kind {
                    crate::decl::DeclKind::EnumLit { ty, .. } => {
                        let ty = name_of(self.types[ty].first_subtype);
                        format!("literal {} of `{ty}`", decl.name)
                    }
                    _ => decl.name.to_string(),
                },
            }
        };
        let entities: Vec<_> = ds.iter().map(|&d| self.entity(d)).collect();
        let short = ds.iter().map(|&d| text(d, false)).collect();
        names_apart(&entities, short, |i| text(ds[i], true))
    }

    pub fn subtype_name(&self, st: crate::ids::SubtypeId) -> String {
        match self.subtypes[st].name {
            Some(n) => n.text.to_string(),
            None => self.type_name(self.base(st)),
        }
    }
}

/// `short[i]`, or `full(i)` if another item with another key reads the same.
fn names_apart<K: PartialEq>(
    keys: &[K],
    short: Vec<String>,
    full: impl Fn(usize) -> String,
) -> Vec<String> {
    (0..keys.len())
        .map(|i| {
            let alike = (0..keys.len())
                .any(|j| keys[j] != keys[i] && short[j].eq_ignore_ascii_case(&short[i]));
            if alike { full(i) } else { short[i].clone() }
        })
        .collect()
}

fn join_path(region: String, name: &str) -> String {
    if region.is_empty() {
        name.to_string()
    } else {
        format!("{region}.{name}")
    }
}

/// `AggResolver` for aggregates inside a complete context: element values go
/// through the same two passes and memo.
struct Resolver {
    scope: ScopeId,
}

impl crate::expr::aggregate::AggResolver for Resolver {
    fn cands(&mut self, db: &mut Db, e: &ast::Expr) -> CandSet {
        db.cands(self.scope, e)
    }

    fn resolve(
        &mut self,
        db: &mut Db,
        e: &ast::Expr,
        exp: Expected,
    ) -> Result<ExprId, crate::Reported> {
        let id = db.resolve(self.scope, e, exp);
        if db.is_poison_expr(id) {
            Err(crate::Reported)
        } else {
            Ok(id)
        }
    }

    fn discrete_range(
        &mut self,
        db: &mut Db,
        r: &ast::DiscreteRange,
        index_ty: TypeId,
    ) -> Result<crate::types::RangeC, crate::Reported> {
        db.discrete_range(self.scope, r, Some(index_ty))
            .map(|(r, _)| r)
    }

    fn type_mark(&mut self, db: &mut Db, e: &ast::Expr) -> Option<crate::ids::SubtypeId> {
        let ast::ExprKind::Name(n) = &e.kind else {
            return None;
        };
        if let Some(st) = db.quiet_type_mark(self.scope, n) {
            return Some(st);
        }
        // A range attribute choice (`v'range => x`) is the subtype that range constrains.
        let (range, ty) = db.range_attr(self.scope, n)?.ok()?;
        let parent = db.types[ty].first_subtype;
        Some(db.subtypes.push(crate::types::SubtypeDef {
            base: ty,
            name: None,
            parent: Some(parent),
            constraint: crate::types::Constraint::Range(range),
            resolution: None,
            span: n.span,
        }))
    }
}

fn class_name(e: &ast::Expr) -> &'static str {
    match &e.kind {
        ast::ExprKind::Aggregate(_) => "an aggregate",
        ast::ExprKind::New(_) => "an allocator",
        ast::ExprKind::Literal(ast::Literal {
            kind: ast::LiteralKind::Null,
            ..
        }) => "`null`",
        _ => "a string literal",
    }
}

impl Db {
    /// A procedure call statement (10.7, 11.4): overload resolution among the
    /// visible procedures, complete argument list in formal order.
    pub(crate) fn proc_call(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Result<crate::hir::Call, crate::Reported> {
        use super::names::{NameRes, Slot, assoc_map};
        let (prefix, args): (&ast::Name, &[ast::AssocElem]) = match &n.kind {
            ast::NameKind::Apply { prefix, args } => (prefix, args),
            _ => (n, &[]),
        };
        let saved = (
            std::mem::take(&mut self.memo),
            std::mem::take(&mut self.name_memo),
        );
        let r = (|| {
            // Method calls `obj.m(..)`: each method with the type of `obj` it selects from.
            let mut owners: Vec<(crate::ids::DeclId, TypeId)> = Vec::new();
            let ds = match self.classify(scope, prefix) {
                NameRes::Overloads(ds) => ds,
                NameRes::One(d) => vec![d],
                NameRes::Value => {
                    let ast::NameKind::Selected {
                        prefix: obj,
                        suffix,
                    } = &prefix.kind
                    else {
                        return Err(self.unsupported(prefix.span, "this procedure name"));
                    };
                    let Ok(ts) = self.prefix_types(scope, obj) else {
                        return Err(crate::Reported);
                    };
                    let key = crate::unit::suffix_key(suffix);
                    for &t in &ts {
                        for d in key.map(|k| self.method_decls(t, k)).unwrap_or_default() {
                            owners.push((d, t));
                        }
                    }
                    if owners.is_empty() {
                        return Err(self.no_method(scope, obj, suffix, &ts, prefix.span));
                    }
                    owners.iter().map(|o| o.0).collect()
                }
                other => {
                    return Err(self.report_name(scope, other, prefix.span, "a procedure name"));
                }
            };
            // A name whose declaration failed was reported there (8.2).
            if ds.iter().any(|&d| {
                matches!(
                    self.decls[self.entity(d)].kind,
                    crate::decl::DeclKind::Error
                )
            }) {
                return Err(crate::Reported);
            }
            let procs: Vec<crate::ids::DeclId> = ds
                .iter()
                .copied()
                .filter(|&d| matches!(self.decls[self.entity(d)].kind.subprogram(), Some(s) if matches!(s.kind, crate::decl::SubprogramKind::Procedure)))
                .collect();
            if procs.is_empty()
                && let Some(f) = ds.iter().copied().find(|&d| {
                    self.decls[self.entity(d)]
                        .kind
                        .subprogram()
                        .is_some_and(|s| s.ret.is_some())
                })
            {
                return Err(self.error(
                    Diagnostic::error(
                        diag::TYPE_MISMATCH,
                        format!(
                            "function `{}` cannot be called as a statement",
                            self.decls[f].name
                        ),
                    )
                    .with_primary(prefix.span, "a function call is an expression")
                    .with_secondary(self.decls[self.entity(f)].span, "declared here")
                    .with_help("assign or use its result, or call a procedure")
                    .with_note("LRM 10.7: a procedure call names a procedure"),
                ));
            }
            if procs.is_empty() {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_AN_OBJECT,
                        format!("`{}` is not a procedure", self.decls[ds[0]].name),
                    )
                    .with_primary(prefix.span, "expected a procedure name")
                    .with_secondary(self.decls[ds[0]].span, "declared here")
                    .with_note("LRM 10.7: a procedure call names a procedure"),
                ));
            }
            let mut best: Vec<(crate::ids::DeclId, u32)> = Vec::new();
            for &d in &procs {
                let e = self.entity(d);
                let Some(s) = self.decls[e].kind.subprogram() else {
                    continue;
                };
                let formals: smallvec::SmallVec<[(yavs_syntax::Symbol, TypeId, bool); 8]> = s
                    .params
                    .iter()
                    .map(|p| (p.name.key, self.base(p.subtype), p.default.is_some()))
                    .collect();
                let Ok(map) = assoc_map(
                    &formals
                        .iter()
                        .map(|f| f.0)
                        .collect::<smallvec::SmallVec<[yavs_syntax::Symbol; 8]>>(),
                    args,
                ) else {
                    continue;
                };
                let mut convs = 0;
                let mut ok = true;
                for (k, (_, ft, has_default)) in formals.iter().enumerate() {
                    match &map[k] {
                        Slot::Parts(parts) => match self.parts_cost(scope, d, k, args, parts) {
                            Some(c) => convs += c,
                            None => ok = false,
                        },
                        Slot::Conv(a) => match self.conv_cost(scope, d, k, args, *a) {
                            Some(c) => convs += c,
                            None => ok = false,
                        },
                        Slot::Whole(a) => match &args[*a].actual {
                            ast::Actual::Expr(x) => {
                                let c = self.cands(scope, x);
                                // An anonymous parameter type takes any type of its class (#283).
                                let cost = if self.anonymous_param(e, *ft) {
                                    self.anonymous_cost(&c, *ft)
                                } else {
                                    self.best_cost(&c, Expected::Type(*ft))
                                };
                                match cost {
                                    Some(c) => convs += c,
                                    None => ok = false,
                                }
                            }
                            ast::Actual::Open(_) if *has_default => {}
                            _ => ok = false,
                        },
                        Slot::None if *has_default => {}
                        Slot::None => ok = false,
                    }
                    if !ok {
                        break;
                    }
                }
                if ok && !best.iter().any(|(b, _)| self.entity(*b) == e) {
                    best.push((d, convs));
                }
            }
            // A method call points at the method name, not at its object.
            let callee_span = match &prefix.kind {
                ast::NameKind::Selected { suffix, .. } if !owners.is_empty() => {
                    crate::unit::suffix_span(suffix, prefix.span)
                }
                _ => prefix.span,
            };
            let Some(min) = best.iter().map(|b| b.1).min() else {
                let mut one: Vec<DeclId> = Vec::new();
                for &d in &procs {
                    let e = self.entity(d);
                    if !one.contains(&e) {
                        one.push(e);
                    }
                }
                if let [p] = one[..]
                    && let Some(r) = self.assoc_mismatch(scope, p, args, callee_span)
                {
                    return Err(r);
                }
                let mut d = Diagnostic::error(
                    diag::NO_OVERLOAD,
                    format!(
                        "no visible procedure `{}` accepts these arguments",
                        self.decls[procs[0]].name
                    ),
                )
                .with_primary(callee_span, "no matching procedure")
                .with_note(
                    "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
                );
                d = self.candidate_labels(d, &procs[..procs.len().min(MAX_CANDIDATE_LABELS)], &[]);
                return Err(self.error(d));
            };
            best.retain(|b| b.1 == min);
            if best.len() > 1 {
                // An actual that already failed fits every overload: its error was reported (8.2).
                let poison = self.poison.ty;
                for a in args {
                    if let ast::Actual::Expr(x) = &a.actual
                        && matches!(self.cands(scope, x),
                            CandSet::Types(is) if is.iter().any(|i| i.ty == poison))
                    {
                        return Err(crate::Reported);
                    }
                }
                let cands: Vec<DeclId> = best.iter().map(|b| b.0).collect();
                let mut d = match self.ambiguous_actual(scope, &cands, args) {
                    Some(d) => d,
                    None => Diagnostic::error(diag::AMBIGUOUS, "this procedure call is ambiguous")
                        .with_primary(prefix.span, "several procedures fit")
                        .with_help("use a selected name such as `pkg.proc`, or qualify an argument")
                        .with_note(
                            "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
                        ),
                };
                for (&c, sig) in cands.iter().zip(self.signatures(&cands, &[])) {
                    d = d.with_secondary(self.decls[c].span, sig);
                }
                return Err(self.error(d));
            }
            let method_of = match (&prefix.kind, owners.iter().find(|o| o.0 == best[0].0)) {
                (ast::NameKind::Selected { prefix: obj, .. }, Some(&(_, t))) => {
                    Some(self.method_object(scope, obj, t))
                }
                _ => None,
            };
            // No call expression: a procedure call is a statement (10.7), so nothing is
            // left unreferenced in the arena.
            let (callee, args, _) = self
                .call_args(scope, best[0].0, args, n.span)
                .ok_or(crate::Reported)?;
            let method_of = self.forwarded_object(best[0].0, method_of);
            Ok(crate::hir::Call {
                callee,
                args,
                method_of,
                span: n.span,
            })
        })();
        (self.memo, self.name_memo) = saved;
        r
    }

    /// No method `obj.suffix`: `protected_member` reports a missing method of
    /// a protected object; anything else does not denote a procedure.
    fn no_method(
        &mut self,
        scope: ScopeId,
        obj: &ast::Name,
        suffix: &ast::Suffix,
        ts: &[TypeId],
        span: Span,
    ) -> crate::Reported {
        // `sv.iv.m` with `iv` not selectable from `sv` (a private variable, E1105).
        if let ast::NameKind::Selected {
            prefix: inner,
            suffix: ast::Suffix::Ident(m),
        } = &obj.kind
            && let Ok(its) = self.prefix_types(scope, inner)
            && let Some((pt, _)) = its.iter().find_map(|&t| self.method_target(t))
            && let Err(r) = self.protected_member(pt, *m, scope)
        {
            return r;
        }
        let target = ts.iter().find_map(|&t| self.method_target(t));
        match (target, suffix) {
            (Some((pt, _)), ast::Suffix::Ident(i)) => match self.protected_member(pt, *i, scope) {
                Err(r) => r,
                // The member exists but is no subprogram: a method whose declaration
                // failed was reported there (8.2), a private variable is no procedure.
                Ok(found)
                    if found.iter().any(|&d| {
                        matches!(
                            self.decls[self.entity(d)].kind,
                            crate::decl::DeclKind::Error
                        )
                    }) =>
                {
                    crate::Reported
                }
                Ok(_) => self.error(
                    Diagnostic::error(diag::NOT_AN_OBJECT, "this name is not a procedure")
                        .with_primary(span, "expected a procedure name")
                        .with_note("LRM 10.7: a procedure call names a procedure"),
                ),
            },
            _ => self.error(
                Diagnostic::error(diag::NOT_AN_OBJECT, "this name is not a procedure")
                    .with_primary(span, "expected a procedure name")
                    .with_secondary(obj.span, "this prefix is not a protected object")
                    .with_note("LRM 10.7: a procedure call names a procedure"),
            ),
        }
    }
}

/// Overload diagnostics of calls (8.3): association errors of the only candidate, calls of
/// procedures as functions, ambiguous actuals.
impl Db {
    /// A secondary label at each candidate with its signature (a note for implicit ones);
    /// `others`: the types the message names besides them.
    fn candidate_labels(&self, mut d: Diagnostic, cs: &[DeclId], others: &[TypeId]) -> Diagnostic {
        for (&c, sig) in cs.iter().zip(self.signatures(cs, others)) {
            let sp = self.decls[c].span;
            d = if sp != Span::default() {
                d.with_secondary(sp, sig)
            } else {
                d.with_note(format!("candidate: {sig}"))
            };
        }
        d
    }

    /// `pr` or `pr(..)` used as a value where every overload is a procedure: E1300.
    fn procedures_only(&mut self, callee: &ast::Name, ds: &[DeclId]) -> bool {
        let procs = !ds.is_empty()
            && ds.iter().all(|&d| {
                matches!(
                    self.decls[self.entity(d)].kind.subprogram(),
                    Some(s) if s.ret.is_none()
                )
            });
        if procs {
            let e = self.entity(ds[0]);
            let d = Diagnostic::error(
                diag::TYPE_MISMATCH,
                format!("procedure `{}` does not return a value", self.decls[e].name),
            )
            .with_primary(callee.span, "a procedure call is a statement")
            .with_secondary(self.decls[e].span, "declared here")
            .with_note("LRM 9.3.4: a function call names a function");
            self.report(d);
        }
        procs
    }

    /// `f(args)` matching no overload `ds` of `f` (E1301), or the association error
    /// of the only function `f` (E1100, E1301, E1310, E1311).
    fn no_function(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        args: &[ast::AssocElem],
        ds: &[DeclId],
    ) {
        let mut fns: Vec<DeclId> = Vec::new();
        for &d in ds {
            let e = self.entity(d);
            if self.decls[e]
                .kind
                .subprogram()
                .is_some_and(|s| s.ret.is_some())
                && !fns.contains(&e)
            {
                fns.push(e);
            }
        }
        if fns.is_empty() && self.procedures_only(prefix, ds) {
            return;
        }
        if let [f] = fns[..]
            && self.assoc_mismatch(scope, f, args, prefix.span).is_some()
        {
            return;
        }
        let name = fns.first().map_or_else(
            || self.source_text(prefix.span),
            |&f| self.decls[f].name.to_string(),
        );
        let mut d = Diagnostic::error(
            diag::NO_OVERLOAD,
            format!("no function `{name}` matches the call"),
        )
        .with_primary(prefix.span, "no matching function")
        .with_note(
            "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
        );
        d = self.candidate_labels(d, &fns[..fns.len().min(MAX_CANDIDATE_LABELS)], &[]);
        if !fns.is_empty() && fns.len() <= MAX_CANDIDATE_LABELS {
            // As the declarations wrote them, unless that reads alike for different ones.
            let mut list: Vec<String> = fns.iter().map(|&f| self.short_signature(f)).collect();
            let alike = (0..list.len()).any(|i| {
                (0..i).any(|j| fns[i] != fns[j] && list[i].eq_ignore_ascii_case(&list[j]))
            });
            if alike {
                list = self.signatures(&fns, &[]);
            }
            let list: Vec<String> = list.iter().map(|s| format!("`{s}`")).collect();
            d = d.with_help(format!("candidates: {}", list.join(", ")));
        }
        self.report(d);
    }

    /// The association list `args` does not fit subprogram `d` itself (6.5.7.1): report
    /// why. `None` when it fits, so only the argument types are wrong.
    fn assoc_mismatch(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        args: &[ast::AssocElem],
        callee: Span,
    ) -> Option<crate::Reported> {
        use super::names::{AssocErr, Slot, assoc_map};
        let e = self.entity(d);
        let s = self.decls[e].kind.subprogram()?;
        let keys: Vec<yavs_syntax::Symbol> = s.params.iter().map(|p| p.name.key).collect();
        let what = match s.kind {
            crate::decl::SubprogramKind::Procedure => "procedure",
            crate::decl::SubprogramKind::Function { .. } => "function",
        };
        let name = self.decls[e].name;
        let formal = |k: usize| {
            let f = args[k].formal.as_ref()?;
            let root = super::names::formal_root(f)?;
            let i = match super::names::conv_formal(f) {
                Some(c) if !keys.contains(&root.key) => c,
                _ => root,
            };
            Some((i, f.span))
        };
        let d = match assoc_map(&keys, args) {
            Ok(map) => {
                // No actual, or `open` (6.5.6.2), for a formal without a default.
                let missing = s.params.iter().zip(&map).find(|(p, m)| {
                    p.default.is_none()
                        && match m {
                            Slot::None => true,
                            Slot::Whole(a) => matches!(args[*a].actual, ast::Actual::Open(_)),
                            Slot::Parts(_) | Slot::Conv(_) => false,
                        }
                });
                let Some((p, m)) = missing else {
                    // Only an actual's type is wrong: report it at that actual (E1300).
                    let typed: Vec<(TypeId, usize)> = s
                        .params
                        .iter()
                        .zip(&map)
                        .filter_map(|(p, m)| match m {
                            Slot::Whole(a) => Some((self.base(p.subtype), *a)),
                            _ => None,
                        })
                        .collect();
                    for (ft, a) in typed {
                        let ast::Actual::Expr(x) = &args[a].actual else {
                            continue;
                        };
                        // Only a name of an object has one fixed type to blame; an overloaded
                        // literal or call in the actual stays E1301 at the callee.
                        let c = self.cands(scope, x);
                        let fixed = matches!(&c, CandSet::Types(is) if !is.is_empty()
                            && is.iter().all(|i| matches!(i.via,
                                Via::Object | Via::Index(_) | Via::Slice(_) | Via::Field(..)
                                | Via::Deref(_))));
                        if fixed && self.best_cost(&c, Expected::Type(ft)).is_none() {
                            self.resolve(scope, x, Expected::Type(ft));
                            return Some(crate::Reported);
                        }
                    }
                    return None;
                };
                let how = if *m != Slot::None {
                    "is open"
                } else {
                    "has no actual"
                };
                Diagnostic::error(
                    diag::MISSING_ACTUAL,
                    format!(
                        "formal `{}` of {what} `{name}` {how} and has no default",
                        p.name.text
                    ),
                )
                .with_primary(callee, format!("no actual for `{}`", p.name.text))
                .with_note(
                    "LRM 6.5.7.1: only a formal with a default may be left out of the association list",
                )
            }
            Err(AssocErr::TooMany(k)) => {
                let n = s.params.len();
                Diagnostic::error(
                    diag::NO_OVERLOAD,
                    format!(
                        "{what} `{name}` takes {n} argument{}, found {}",
                        if n == 1 { "" } else { "s" },
                        args.len()
                    ),
                )
                .with_primary(args[k].span, "extra argument")
                .with_note(
                    "LRM 6.5.7.1: a positional actual corresponds to the interface element at the same position",
                )
            }
            Err(AssocErr::Unknown(k)) => {
                let (i, sp) = formal(k)?;
                Diagnostic::error(
                    diag::NOT_DECLARED,
                    format!("{what} `{name}` has no parameter `{}`", i.text),
                )
                .with_primary(sp, "no such parameter")
                .with_note("LRM 6.5.7.1: a formal designator names a formal of this interface list")
            }
            Err(AssocErr::Twice(k)) => {
                let (i, sp) = formal(k)?;
                let pos = keys.iter().position(|&x| x == i.key)?;
                let first = (0..k).find(|&j| match &args[j].formal {
                    None => j == pos,
                    Some(_) => formal(j).is_some_and(|(f, _)| f.key == i.key),
                })?;
                Diagnostic::error(
                    diag::FORMAL_TWICE,
                    format!("formal `{}` is associated twice", i.text),
                )
                .with_primary(sp, "associated again here")
                .with_secondary(args[first].span, "first associated here")
                .with_note(
                    "LRM 6.5.7.1: every scalar subelement of a formal is associated exactly once",
                )
            }
            Err(AssocErr::Other) => return None,
        };
        let d = self.candidate_labels(d, &[e], &[]);
        Some(self.error(d))
    }

    /// Procedure call candidates `cands` that differ only in the type of one actual:
    /// E1302 at that actual, with a qualification as help.
    fn ambiguous_actual(
        &mut self,
        scope: ScopeId,
        cands: &[DeclId],
        args: &[ast::AssocElem],
    ) -> Option<Diagnostic> {
        use super::names::{Slot, assoc_map};
        // Per candidate, the formal (decl, subtype) of each association.
        let mut per: Vec<Vec<(Option<Span>, crate::ids::SubtypeId)>> = Vec::new();
        for &c in cands {
            let s = self.decls[self.entity(c)].kind.subprogram()?;
            let keys: Vec<yavs_syntax::Symbol> = s.params.iter().map(|p| p.name.key).collect();
            let map = assoc_map(&keys, args).ok()?;
            let mut f = vec![(None, self.poison.subtype); args.len()];
            for (k, m) in map.iter().enumerate() {
                let at: &[usize] = match m {
                    Slot::None => &[],
                    Slot::Whole(a) | Slot::Conv(a) => std::slice::from_ref(a),
                    Slot::Parts(v) => v,
                };
                for &a in at {
                    f[a] = (s.params[k].mark, s.params[k].subtype);
                }
            }
            per.push(f);
        }
        let mut differ = (0..args.len()).filter(|&a| {
            per.iter()
                .any(|f| self.base(f[a].1) != self.base(per[0][a].1))
        });
        let a = differ.next()?;
        if differ.next().is_some() {
            return None;
        }
        let ast::Actual::Expr(x) = &args[a].actual else {
            return None;
        };
        let mut inner = x;
        while let ast::ExprKind::Paren(i) = &inner.kind {
            inner = i;
        }
        let text = self.source_text(x.span);
        use ast::LiteralKind as L;
        let (msg, what) = match &inner.kind {
            ast::ExprKind::Aggregate(_) => ("this aggregate is ambiguous".to_string(), "aggregate"),
            ast::ExprKind::Literal(l) => match &l.kind {
                L::Char(_) => (
                    format!("character literal `{text}` is ambiguous"),
                    "literal",
                ),
                L::String(_) | L::BitString(_) => ("string literal is ambiguous".into(), "literal"),
                L::Null => ("`null` is ambiguous".into(), "literal"),
                L::Abstract(_) | L::Physical { .. } => {
                    ("numeric literal is ambiguous".into(), "literal")
                }
            },
            ast::ExprKind::Name(_) => match self.cands(scope, x) {
                CandSet::Types(is) if is.iter().all(|i| matches!(i.via, Via::EnumLit(_))) => (
                    format!("enumeration literal `{text}` is ambiguous"),
                    "literal",
                ),
                CandSet::Types(is) if is.iter().all(|i| matches!(i.via, Via::Call(_))) => {
                    (format!("call of `{text}` is ambiguous"), "call")
                }
                _ => (format!("`{text}` is ambiguous"), "expression"),
            },
            _ => ("this argument is ambiguous".into(), "expression"),
        };
        let (fd, fst) = per[0][a];
        let mark = self.written(fd, fst);
        let q = if text.starts_with('(') {
            format!("{mark}'{text}")
        } else {
            format!("{mark}'({text})")
        };
        Some(
            Diagnostic::error(diag::AMBIGUOUS, msg)
                .with_primary(x.span, "fits several procedures")
                .with_help(format!("qualify the {what}: `{q}`"))
                .with_note(
                    "LRM 12.5: the occurrence is legal only with exactly one interpretation in its complete context",
                ),
        )
    }

    /// The type mark of a parameter or function result as its declaration wrote it
    /// (`integer`, not `INTEGER`), else the name of subtype `st`.
    fn written(&self, mark: Option<Span>, st: crate::ids::SubtypeId) -> String {
        match mark {
            Some(sp) => self.source_text(sp),
            None => self.subtype_name(st),
        }
    }

    /// `f (x: integer) return integer`: a candidate in a help line, as written.
    fn short_signature(&self, d: DeclId) -> String {
        let e = self.entity(d);
        let decl = &self.decls[e];
        let Some(s) = decl.kind.subprogram() else {
            return decl.name.to_string();
        };
        let params: Vec<String> = s
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name.text, self.written(p.mark, p.subtype)))
            .collect();
        let mut out = decl.name.to_string();
        if !params.is_empty() {
            out += &format!(" ({})", params.join("; "));
        }
        if let Some(r) = s.ret {
            out += &format!(" return {}", self.written(s.ret_mark, r));
        }
        out
    }

    /// 8.3: an operator applied to a numeric_std/numeric_bit vector and an integer.
    fn vector_integer_hint(&mut self, scope: ScopeId, e: &ast::Expr) -> Option<String> {
        let ast::ExprKind::Binary { left, right, .. } = &e.kind else {
            return None;
        };
        let types = |db: &mut Db, x: &ast::Expr| match db.cands(scope, x) {
            CandSet::Types(is) => is.iter().map(|i| i.ty).collect(),
            CandSet::Class(_) | CandSet::Alloc(_) => Vec::new(),
        };
        let (l, r) = (types(self, left), types(self, right));
        // ponytail: keyed on the type names of numeric_std/numeric_bit; a table of known
        // conversions (8.3 "type mismatch") if more hints are wanted.
        let vector = |db: &Db, ts: &[TypeId]| {
            ts.iter().find_map(|&t| {
                let n = db.type_name(t).to_ascii_lowercase();
                if n.ends_with("unsigned") {
                    Some("to_unsigned")
                } else if n.ends_with("signed") {
                    Some("to_signed")
                } else {
                    None
                }
            })
        };
        let integer = |db: &Db, ts: &[TypeId]| {
            ts.iter().any(|&t| {
                matches!(
                    db.types[t].kind,
                    TypeKind::Integer | TypeKind::UniversalInteger
                )
            })
        };
        let f = match (vector(self, &l), vector(self, &r)) {
            (Some(f), None) if integer(self, &r) => f,
            (None, Some(f)) if integer(self, &l) => f,
            _ => return None,
        };
        Some(format!("use `{f}(x, n)` to convert an integer to a vector"))
    }
}

/// Number of elements of `l dir r` (0 for a null range).
fn len(l: i64, d: crate::types::Direction, r: i64) -> i64 {
    let (lo, hi) = if d == crate::types::Direction::To {
        (l, r)
    } else {
        (r, l)
    };
    if hi < lo {
        0
    } else {
        hi.saturating_sub(lo).saturating_add(1)
    }
}
