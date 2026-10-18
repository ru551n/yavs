//! Top-down selection and HIR construction (sem.md 3.2, 3.6, 3.7).

use rsim_diag::{Diagnostic, Span};
use rsim_syntax::ast;

use super::{CandSet, Expected, Interp, Via};
use crate::db::Db;
use crate::decl::operator_key;
use crate::diag;
use crate::hir::{Actual, Arg, Expr, ExprKind};
use crate::ids::{ExprId, ScopeId, TypeId};
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
        self.expr(scope, e, Expected::Type(t))
    }

    /// Resolve a complete context whose value is required at analysis time
    /// (type bounds, unit scales, choices): static overflow is error `E1401`.
    pub(crate) fn static_expr(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected) -> ExprId {
        self.complete_context(scope, e, exp, true)
    }

    fn complete_context(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        exp: Expected,
        required: bool,
    ) -> ExprId {
        let saved = std::mem::take(&mut self.memo);
        let saved_names = std::mem::take(&mut self.name_memo);
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
        for o in std::mem::replace(&mut self.overflow, saved_overflow) {
            let span = self.exprs[o].span;
            let d = if required {
                Diagnostic::error(diag::OVERFLOW, "static arithmetic overflows 64 bits")
                    .with_primary(span, "this value overflows")
            } else {
                Diagnostic::warning(diag::W_OVERFLOW, "static arithmetic overflows 64 bits")
                    .with_primary(span, "this value overflows")
                    .with_note("the value is not needed during analysis; the run-time check stays")
            };
            self.report(d);
        }
        self.memo = saved;
        self.name_memo = saved_names;
        id
    }

    /// Select the interpretation of `e` that fits `exp` and build its HIR.
    pub(crate) fn resolve(&mut self, scope: ScopeId, e: &ast::Expr, exp: Expected) -> ExprId {
        if let ast::ExprKind::Paren(inner) = &e.kind {
            return self.resolve(scope, inner, exp);
        }
        let cs = self.cands(scope, e);
        match cs {
            CandSet::Class(m) => match exp {
                Expected::Type(t) if self.class_admits(m, t) => self.build_class(scope, e, t),
                Expected::Type(t) => {
                    let what = class_name(e);
                    let tn = self.type_name(t);
                    self.report(
                        Diagnostic::error(
                            diag::TYPE_MISMATCH,
                            format!("expected `{tn}`, found {what}"),
                        )
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
            CandSet::Types(is) => {
                if is.iter().any(|i| i.ty == self.poison.ty) {
                    return self.build_any(scope, e, is[0], exp);
                }
                let mut best: Vec<(Interp, u32)> = Vec::new();
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
            K::Literal(l) => match crate::expr::literal::literal_expr(self, l, i.ty) {
                Ok(id) => id,
                Err(_) => self.poison_expr(),
            },
            K::Paren(inner) => self.build(scope, inner, i),
            K::Name(n) => self.build_name(scope, n, i, e.span),
            K::Unary { operand, .. } => self.build_operator(scope, i, &[operand.as_ref()], e.span),
            K::Binary { left, right, .. } => {
                self.build_operator(scope, i, &[left.as_ref(), right.as_ref()], e.span)
            }
            K::Qualified { type_mark, operand } => {
                let Ok(st) = self.type_mark(scope, type_mark) else {
                    return self.poison_expr();
                };
                let t = self.base(st);
                let op = self.resolve(scope, operand, Expected::Type(t));
                let (stat, value) = (
                    self.exprs[op].stat.min(self.subtype_staticness(st)),
                    self.exprs[op].value,
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
            K::Conditional(_) => {
                self.unsupported(e.span, "conditional expressions", "S7");
                self.poison_expr()
            }
            K::Unaffected | K::Error => self.poison_expr(),
        }
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
        let Some(s) = self.decls[callee].kind.subprogram() else {
            return self.poison_expr();
        };
        let formals: Vec<TypeId> = s.params.iter().map(|p| self.base(p.subtype)).collect();
        let ret = s.ret.unwrap_or(self.poison.subtype);
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        let mut stat = if implicit {
            Staticness::Locally
        } else {
            Staticness::None
        };
        let mut hargs = Vec::new();
        for (k, (o, f)) in operands.iter().zip(formals).enumerate() {
            let id = self.resolve(scope, o, Expected::Type(f));
            stat = stat.min(self.exprs[id].stat);
            hargs.push(Arg {
                formal: k as u16,
                actual: Actual::Expr(id),
            });
        }
        let id = self.add_expr(Expr {
            kind: ExprKind::Call {
                callee,
                args: hargs.into(),
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
                crate::expr::literal::literal_expr(self, l, t).unwrap_or(self.poison.expr)
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
            _ => {
                self.unsupported(e.span, "allocators", "S6");
                self.poison_expr()
            }
        }
    }

    /// 9.2.9: a condition of a type other than BOOLEAN goes through a visible `??`.
    fn implicit_condition(&mut self, scope: ScopeId, e: &ast::Expr, is: &[Interp]) -> ExprId {
        let boolean = self.well_known.boolean;
        let mut found = Vec::new();
        if let Visible::Overloads(ds) = self.lookup(scope, operator_key("??")) {
            for d in ds {
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
                    && !found.iter().any(|(_, t)| *t == pt)
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
                    | NameRes::Error
            ) {
                self.report_name(scope, r);
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
                if crate::attr::PredefAttr::from_key(attr.key).is_none() {
                    self.report(
                        Diagnostic::error(
                            diag::UNKNOWN_ATTRIBUTE,
                            format!("unknown attribute `{}`", attr.text),
                        )
                        .with_primary(attr.span, "not a predefined attribute"),
                    );
                } else {
                    self.unsupported(n.span, "this use of a predefined attribute", "S3");
                }
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
            for o in operands {
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
                let mut d = Diagnostic::error(
                    diag::NO_OVERLOAD,
                    format!("no visible `\"{sym}\"` accepts these operands"),
                )
                .with_primary(e.span, format!("no matching {what}"));
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
                kind: ast::NameKind::Apply { prefix, .. },
                ..
            }) = &e.kind
            {
                self.report(
                    Diagnostic::error(
                        diag::NO_OVERLOAD,
                        "no visible subprogram accepts these arguments",
                    )
                    .with_primary(prefix.span, "no matching subprogram"),
                );
                return;
            }
            self.report(
                Diagnostic::error(diag::NOT_AN_OBJECT, "this name does not denote a value")
                    .with_primary(e.span, "not an object, literal or function"),
            );
            return;
        }
        let found: Vec<String> = is
            .iter()
            .map(|i| format!("`{}`", self.type_name(i.ty)))
            .collect();
        let want = match exp {
            Expected::Type(t) => format!("`{}`", self.type_name(t)),
            Expected::Condition => "BOOLEAN".into(),
            Expected::Discrete => "a discrete type".into(),
            Expected::Any => "a value".into(),
        };
        self.report(
            Diagnostic::error(
                diag::TYPE_MISMATCH,
                format!("expected {want}, found {}", found.join(" or ")),
            )
            .with_primary(e.span, format!("expected {want}")),
        );
    }

    fn ambiguous(&mut self, e: &ast::Expr, best: &[Interp]) {
        let mut d = Diagnostic::error(diag::AMBIGUOUS, "this expression is ambiguous")
            .with_primary(e.span, "several interpretations fit");
        for i in best {
            match i.via {
                Via::Call(c) | Via::EnumLit(c) => {
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
        self.report(
            d.with_help("qualify an operand (`t'(x)`) or use a selected name (`pkg.\"+\"`)"),
        );
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
        use super::names::{NameRes, assoc_map};
        let (prefix, args): (&ast::Name, &[ast::AssocElem]) = match &n.kind {
            ast::NameKind::Apply { prefix, args } => (prefix, args),
            _ => (n, &[]),
        };
        let saved = (
            std::mem::take(&mut self.memo),
            std::mem::take(&mut self.name_memo),
        );
        let r = (|| {
            let ds = match self.classify(scope, prefix) {
                NameRes::Overloads(ds) => ds,
                NameRes::One(d) => vec![d],
                NameRes::Value => {
                    return Err(self.unsupported(prefix.span, "protected method calls", "S6"));
                }
                other => return Err(self.report_name(scope, other)),
            };
            let procs: Vec<crate::ids::DeclId> = ds
                .iter()
                .copied()
                .filter(|&d| matches!(self.decls[self.entity(d)].kind.subprogram(), Some(s) if matches!(s.kind, crate::decl::SubprogramKind::Procedure)))
                .collect();
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
                let formals: Vec<(rsim_syntax::Symbol, TypeId, bool)> = s
                    .params
                    .iter()
                    .map(|p| (p.name.key, self.base(p.subtype), p.default.is_some()))
                    .collect();
                let Some(map) = assoc_map(&formals.iter().map(|f| f.0).collect::<Vec<_>>(), args)
                else {
                    continue;
                };
                let mut convs = 0;
                let mut ok = true;
                for (k, (_, ft, has_default)) in formals.iter().enumerate() {
                    match map[k] {
                        Some(a) => match &args[a].actual {
                            ast::Actual::Expr(x) => {
                                let c = self.cands(scope, x);
                                match self.best_cost(&c, Expected::Type(*ft)) {
                                    Some(c) => convs += c,
                                    None => ok = false,
                                }
                            }
                            ast::Actual::Open(_) => {}
                            _ => ok = false,
                        },
                        None if *has_default => {}
                        None => ok = false,
                    }
                    if !ok {
                        break;
                    }
                }
                if ok && !best.iter().any(|(b, _)| self.entity(*b) == e) {
                    best.push((d, convs));
                }
            }
            let Some(min) = best.iter().map(|b| b.1).min() else {
                let mut d = Diagnostic::error(
                    diag::NO_OVERLOAD,
                    format!(
                        "no visible procedure `{}` accepts these arguments",
                        self.decls[procs[0]].name
                    ),
                )
                .with_primary(prefix.span, "no matching procedure");
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
                let mut d = Diagnostic::error(diag::AMBIGUOUS, "this procedure call is ambiguous")
                    .with_primary(prefix.span, "several procedures fit");
                for (c, _) in &best {
                    let sig = self.signature(*c);
                    d = d.with_secondary(self.decls[*c].span, sig);
                }
                return Err(self.error(
                    d.with_help("use a selected name such as `pkg.proc`, or qualify an argument"),
                ));
            }
            let id = self.build_call(scope, best[0].0, args, n.span, None);
            if self.is_poison_expr(id) {
                return Err(crate::Reported);
            }
            // The call expression only carried the arguments; it is not referenced.
            match std::mem::replace(&mut self.exprs[id].kind, ExprKind::Error) {
                ExprKind::Call {
                    callee,
                    args,
                    method_of,
                    ..
                } => Ok(crate::hir::Call {
                    callee,
                    args,
                    method_of,
                    span: n.span,
                }),
                _ => Err(crate::Reported),
            }
        })();
        (self.memo, self.name_memo) = saved;
        r
    }
}
