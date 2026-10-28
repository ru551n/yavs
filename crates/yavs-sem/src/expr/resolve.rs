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
        let name = self.subtype_name(st);
        self.report(
            Diagnostic::warning(
                diag::W_STATIC_OUTSIDE,
                format!("static value is outside subtype `{name}`"),
            )
            .with_primary(span, "out of range")
            .with_note(
                "the code may never run, so this is checked again at run time (LRM 9.3.5, 9.3.6)",
            ),
        );
        true
    }

    /// `E1403`: a static scalar value outside the static subtype it goes into (4.3).
    pub(crate) fn check_in_subtype(&mut self, id: ExprId, st: crate::ids::SubtypeId) {
        if let Some(crate::stat::ConstValue::Array(a)) = self.value(id)
            && let crate::types::Constraint::Array {
                dims: Some(dims), ..
            } = &self.subtypes[st].constraint
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
            // 4.3: both lengths static, so the run-time length check would always fail.
            let span = self.exprs[id].span;
            self.report(
                Diagnostic::error(
                    diag::LENGTH_MISMATCH,
                    format!("length mismatch: the target has {want} elements, the value {got}"),
                )
                .with_primary(span, format!("{got} elements")),
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
                other => format!("{other:?}"),
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
        for (o, why) in std::mem::replace(&mut self.overflow, saved_overflow) {
            let span = self.exprs[o].span;
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
            self.report(d);
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
        if let ast::ExprKind::Paren(inner) = &e.kind {
            return self.resolve(scope, inner, exp);
        }
        // With a type from context, each alternative is checked against it: precise errors.
        if let (ast::ExprKind::Conditional(alts), Expected::Type(t)) = (&e.kind, exp) {
            return self.conditional(scope, e, alts, t);
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
                        return self.poison_expr();
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
                            .with_help("qualify it with a type mark, as in `t'(...)`"),
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
                    let tn = self.type_name(t);
                    let dn = self.type_name(d);
                    self.report(
                        Diagnostic::error(
                            diag::TYPE_MISMATCH,
                            format!("expected `{tn}`, found an allocator of `{dn}`"),
                        )
                        .with_primary(
                            e.span,
                            format!("an allocator of `{dn}` cannot be of type `{tn}`"),
                        ),
                    );
                    self.poison_expr()
                }
                _ => {
                    self.report(
                        Diagnostic::error(
                            diag::AMBIGUOUS,
                            "the access type of this allocator cannot be determined from its context",
                        )
                        .with_primary(e.span, "type not determined")
                        .with_help("qualify it with a type mark, as in `t'(...)`"),
                    );
                    self.poison_expr()
                }
            },
            CandSet::Types(is) => {
                if is.iter().any(|i| i.ty == self.poison.ty) {
                    return self.build_any(scope, e, is[0], exp);
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
                    return self.implicit_condition(scope, e, &is);
                }
                let Some(min) = best.iter().map(|(_, c)| *c).min() else {
                    self.no_interp(scope, e, exp, &is);
                    return self.poison_expr();
                };
                best.retain(|(_, c)| *c == min);
                if best.len() > 1 {
                    self.prefer_explicit(&mut best);
                }
                if best.len() > 1 {
                    self.ambiguous(e, &best.iter().map(|b| b.0).collect::<Vec<_>>());
                    return self.poison_expr();
                }
                self.build_any(scope, e, best[0].0, exp)
            }
        }
    }

    /// Build the chosen interpretation and insert the implicit conversion (2.8).
    fn build_any(&mut self, scope: ScopeId, e: &ast::Expr, i: Interp, exp: Expected) -> ExprId {
        let id = self.build(scope, e, i);
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
                    && self.in_std_library(scope)
                    && let Some(v) = crate::expr::literal::negated_min(self, l, i.ty)
                {
                    return crate::expr::literal::const_expr(self, e.span, i.ty, Some(v));
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
                self.unsupported(e.span, "aggregates", "S3");
                self.poison_expr()
            }
            K::New(_) => {
                self.unsupported(e.span, "allocators", "S6");
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
        let Via::Call(d) = i.via else {
            return self.poison_expr();
        };
        let callee = self.entity(d);
        if self.decls[callee].kind.subprogram().is_none() {
            return self.poison_expr();
        }
        // A user operator with anonymous parameter types goes to its instance for the
        // operands' types (#308).
        let actuals: smallvec::SmallVec<[Option<&ast::Expr>; 2]> =
            operands.iter().map(|o| Some(*o)).collect();
        let Some((callee, pre)) = self.implicit_instance_of(scope, callee, &actuals, span, span)
        else {
            return self.poison_expr();
        };
        let Some(s) = self.decls[callee].kind.subprogram() else {
            return self.poison_expr();
        };
        let formals: smallvec::SmallVec<[TypeId; 2]> =
            s.params.iter().map(|p| self.base(p.subtype)).collect();
        let ret = s.ret.unwrap_or(self.poison.subtype);
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        // 9.4.3: a pure function call with globally static actuals is globally static
        // (sem folds only the implicit operations, so user calls never become locally static).
        let pure = matches!(s.kind, crate::decl::SubprogramKind::Function { pure: true });
        let mut stat = if implicit {
            Staticness::Locally
        } else if pure {
            Staticness::Globally
        } else {
            Staticness::None
        };
        let mut hargs: smallvec::SmallVec<[Arg; 2]> = smallvec::SmallVec::new();
        for (k, (o, f)) in operands.iter().zip(formals).enumerate() {
            let id = match pre.get(&k) {
                Some(&id) => id,
                None => self.resolve(scope, o, Expected::Type(f)),
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
                    _ => self.types[t].first_subtype,
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
            [] => {
                let found = is.first().map(|i| self.type_name(i.ty));
                let mut d = Diagnostic::error(diag::NOT_BOOLEAN, "condition is not BOOLEAN")
                    .with_primary(
                        e.span,
                        match &found {
                            Some(t) => format!("this is of type `{t}`"),
                            None => "not a BOOLEAN expression".into(),
                        },
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
                .with_primary(e.span, "ambiguous condition");
                for (op, _) in &found {
                    let sig = self.signature(*op);
                    d = d.with_secondary(self.decls[*op].span, sig);
                }
                self.report(d);
                self.poison_expr()
            }
        }
    }

    /// Report why `e` has no interpretation that fits `exp`.
    fn no_interp(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected, is: &[Interp]) {
        use ast::ExprKind as K;
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
                    self.unsupported(n.span, "this use of a predefined attribute", "S3");
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
                .with_primary(e.span, format!("no matching {what}"));
                if let Some(h) = self.vector_integer_hint(scope, e) {
                    d = d.with_help(h);
                }
                if let Visible::Overloads(ds) = self.lookup(scope, operator_key(sym)) {
                    let n = ds.len();
                    for &c in ds.iter().take(MAX_CANDIDATE_LABELS) {
                        let sig = self.signature(c);
                        let sp = self.decls[c].span;
                        if sp != Span::default() {
                            d = d.with_secondary(sp, sig);
                        } else {
                            d = d.with_note(format!("candidate: {sig}"));
                        }
                    }
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
                            .with_secondary(prefix.span, format!("of type `{tn}`")),
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
                            .with_primary(first.span, format!("{} indexes", args.len())),
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
                    .with_primary(prefix.span, "no matching subprogram"),
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
                            .with_primary(prefix.span, "no matching subprogram"),
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
            let tn = self.type_name(t);
            let mut d = Diagnostic::error(
                diag::NO_OVERLOAD,
                format!("no function `{}` returns `{tn}`", self.decls[calls[0]].name),
            )
            .with_primary(e.span, format!("expected `{tn}`"));
            for &c in calls.iter().take(MAX_CANDIDATE_LABELS) {
                d = self.candidate_label(d, c);
            }
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
        .with_primary(e.span, format!("expected {want}"));
        if let ([i], Expected::Type(t)) = (is, exp)
            && let Some(h) = self.conversion_hint(i.ty, t)
        {
            d = d.with_help(h);
        }
        self.report(d);
    }

    /// 8.3: how to turn a value of type `from` into the expected type `to`: a type
    /// conversion between closely related types (9.3.6), `to_unsigned`/`to_signed` from an
    /// integer.
    fn conversion_hint(&self, from: TypeId, to: TypeId) -> Option<String> {
        // ponytail: keyed on the numeric_std/numeric_bit names like `vector_integer_hint`;
        // `UNRESOLVED_UNSIGNED` is shown as the subtype `unsigned` users write.
        let name = self.type_name(to);
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
    /// in the same package or made visible by a use clause): the explicit ones win, as
    /// in sim1, sim2 and sim3 (decisions.md).
    fn prefer_explicit(&self, best: &mut smallvec::SmallVec<[(Interp, u32); 4]>) {
        let calls: Option<Vec<DeclId>> = best
            .iter()
            .map(|(i, _)| match i.via {
                Via::Call(d) => Some(self.entity(d)),
                _ => None,
            })
            .collect();
        let Some(calls) = calls else { return };
        let (imp, exp): (Vec<DeclId>, Vec<DeclId>) =
            calls.iter().partition(|&&d| self.decls[d].implicit);
        if imp.is_empty() || exp.is_empty() {
            return;
        }
        best.retain(|(i, _)| matches!(i.via, Via::Call(d) if !self.decls[self.entity(d)].implicit));
    }

    fn ambiguous(&mut self, e: &ast::Expr, best: &[Interp]) {
        let mut d = Diagnostic::error(diag::AMBIGUOUS, "this expression is ambiguous")
            .with_primary(e.span, "several interpretations fit");
        for i in best {
            match i.via {
                Via::Call(c) | Via::Method(c, _) | Via::EnumLit(c) => {
                    let sig = self.signature(c);
                    let sp = self.decls[c].span;
                    d = if sp != Span::default() {
                        d.with_secondary(sp, sig)
                    } else {
                        d.with_note(format!("candidate: {sig}"))
                    };
                }
                _ => d = d.with_note(format!("could be of type `{}`", self.type_name(i.ty))),
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

    /// Names of `ts` for one message: types whose simple names collide (the
    /// same type declared by two generic instances) are qualified by their region.
    pub fn type_names(&self, ts: &[TypeId]) -> Vec<String> {
        let simple: Vec<String> = ts.iter().map(|&t| self.type_name(t)).collect();
        ts.iter()
            .enumerate()
            .map(|(i, &t)| {
                let clash = ts
                    .iter()
                    .enumerate()
                    .any(|(j, &u)| u != t && simple[j].eq_ignore_ascii_case(&simple[i]));
                if !clash {
                    return simple[i].clone();
                }
                let scope = self.decls[self.types[t].decl].scope;
                let region = crate::mangle::region(self, scope);
                if region.is_empty() {
                    simple[i].clone()
                } else {
                    format!("{region}.{}", simple[i])
                }
            })
            .collect()
    }

    /// `function "+" (l: unsigned; r: natural) return unsigned` (8.3), original spelling.
    pub fn signature(&self, d: crate::ids::DeclId) -> String {
        let e = self.entity(d);
        let decl = &self.decls[e];
        match decl.kind.subprogram() {
            Some(s) => {
                let params: Vec<String> = s
                    .params
                    .iter()
                    .map(|p| format!("{}: {}", p.name.text, self.subtype_name(p.subtype)))
                    .collect();
                let head = match s.kind {
                    crate::decl::SubprogramKind::Procedure => "procedure",
                    crate::decl::SubprogramKind::Function { .. } => "function",
                };
                let mut out = format!("{head} {}", decl.name);
                if !params.is_empty() {
                    out += &format!(" ({})", params.join("; "));
                }
                if let Some(r) = s.ret {
                    out += &format!(" return {}", self.subtype_name(r));
                }
                out
            }
            None => match decl.kind {
                crate::decl::DeclKind::EnumLit { ty, .. } => {
                    format!("literal {} of `{}`", decl.name, self.type_name(ty))
                }
                _ => decl.name.to_string(),
            },
        }
    }

    pub fn subtype_name(&self, st: crate::ids::SubtypeId) -> String {
        match self.subtypes[st].name {
            Some(n) => n.text.to_string(),
            None => self.type_name(self.base(st)),
        }
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
                        return Err(self.unsupported(prefix.span, "this procedure name", "S6"));
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
                    .with_help("assign or use its result, or call a procedure"),
                ));
            }
            if procs.is_empty() {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_AN_OBJECT,
                        format!("`{}` is not a procedure", self.decls[ds[0]].name),
                    )
                    .with_primary(prefix.span, "expected a procedure name")
                    .with_secondary(self.decls[ds[0]].span, "declared here"),
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
                .with_primary(callee_span, "no matching procedure");
                for &c in procs.iter().take(MAX_CANDIDATE_LABELS) {
                    let sig = self.signature(c);
                    let sp = self.decls[c].span;
                    d = if sp != Span::default() {
                        d.with_secondary(sp, sig)
                    } else {
                        d.with_note(format!("candidate: {sig}"))
                    };
                }
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
                        .with_help(
                            "use a selected name such as `pkg.proc`, or qualify an argument",
                        ),
                };
                for c in cands {
                    let sig = self.signature(c);
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
                        .with_primary(span, "expected a procedure name"),
                ),
            },
            _ => self.error(
                Diagnostic::error(diag::NOT_AN_OBJECT, "this name is not a procedure")
                    .with_primary(span, "expected a procedure name")
                    .with_secondary(obj.span, "this prefix is not a protected object"),
            ),
        }
    }
}

/// Overload diagnostics of calls (8.3): association errors of the only candidate, calls of
/// procedures as functions, ambiguous actuals.
impl Db {
    /// A secondary label at candidate `c` with its signature (a note for implicit ones).
    fn candidate_label(&self, d: Diagnostic, c: DeclId) -> Diagnostic {
        let sig = self.signature(c);
        let sp = self.decls[c].span;
        if sp != Span::default() {
            d.with_secondary(sp, sig)
        } else {
            d.with_note(format!("candidate: {sig}"))
        }
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
            .with_secondary(self.decls[e].span, "declared here");
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
        .with_primary(prefix.span, "no matching function");
        for &f in fns.iter().take(MAX_CANDIDATE_LABELS) {
            d = self.candidate_label(d, f);
        }
        if !fns.is_empty() && fns.len() <= MAX_CANDIDATE_LABELS {
            let list: Vec<String> = fns
                .iter()
                .map(|&f| format!("`{}`", self.short_signature(f)))
                .collect();
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
            }
            Err(AssocErr::Unknown(k)) => {
                let (i, sp) = formal(k)?;
                Diagnostic::error(
                    diag::NOT_DECLARED,
                    format!("{what} `{name}` has no parameter `{}`", i.text),
                )
                .with_primary(sp, "no such parameter")
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
            }
            Err(AssocErr::Other) => return None,
        };
        let d = self.candidate_label(d, e);
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
                .with_help(format!("qualify the {what}: `{q}`")),
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
