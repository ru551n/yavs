//! Name interpretation: simple and expanded names, `Apply` (calls, conversions,
//! attribute functions), predefined attributes (sem.md 1.5, 3.3).
//!
//! Both passes go through [`Db::classify`], which resolves the non-overloaded
//! prefix chain of a name without reporting; the top-down pass reports.

use rsim_diag::{Diagnostic, Span};
use rsim_syntax::Symbol;
use rsim_syntax::ast::{self, Ident};

use super::{CandSet, Expected, Interp, Via};
use crate::attr::PredefAttr;
use crate::db::{Db, Reported};
use crate::decl::{Alias, DeclKind, char_key, operator_key};
use crate::diag;
use crate::hir::{Actual, Arg, AttrPrefix, Expr, ExprKind};
use crate::ids::{DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use crate::scope::Visible;
use crate::stat::{ConstValue, Staticness};
use crate::types::{Direction, TypeKind};
use crate::unit::{Region, suffix_key, suffix_span};

/// What a name without arguments denotes.
pub(crate) enum NameRes {
    One(DeclId),
    Overloads(Vec<DeclId>),
    /// Nothing visible: report `E1100` (or the conflict) at `ident`.
    Missing(Ident),
    Conflict(Ident, crate::scope::Conflict),
    /// A construct of a later slice.
    Unsupported(Span, &'static str, &'static str),
    /// The suffix is not declared in the package region.
    NotInRegion(ScopeId, Span, Symbol),
    /// The name denotes a value computed from an object or call (selection,
    /// indexing): interpreted by `name_cands`, not by declaration lookup.
    Value,
    /// Already reported.
    Error,
}

fn name_ident(n: &ast::Name) -> Option<(Symbol, Ident)> {
    match &n.kind {
        ast::NameKind::Simple(i) => Some((i.key, *i)),
        ast::NameKind::Operator(op) => Some((
            operator_key(op.as_str()),
            Ident {
                text: Symbol::intern(&format!("\"{op}\"")),
                key: operator_key(op.as_str()),
                span: n.span,
            },
        )),
        ast::NameKind::Character(c) => Some((
            char_key(*c),
            Ident {
                text: Symbol::intern(&format!("'{c}'")),
                key: char_key(*c),
                span: n.span,
            },
        )),
        _ => None,
    }
}

impl Db {
    /// Resolve a simple or expanded name (no arguments, no attribute).
    pub(crate) fn classify(&mut self, scope: ScopeId, n: &ast::Name) -> NameRes {
        if let Some((key, ident)) = name_ident(n) {
            return match self.lookup(scope, key) {
                Visible::One(d) => NameRes::One(d),
                Visible::Overloads(ds) => NameRes::Overloads(ds),
                Visible::None => NameRes::Missing(ident),
                Visible::Hidden(c) => NameRes::Conflict(ident, c),
            };
        }
        match &n.kind {
            ast::NameKind::Selected { prefix, suffix } => {
                let Some(key) = suffix_key(suffix) else {
                    return NameRes::Unsupported(n.span, "`.all`", "S6");
                };
                match self.classify(scope, prefix) {
                    NameRes::One(d) => {
                        let region = match self.decls[self.entity(d)].kind {
                            DeclKind::Library(l) => Region::Library(l),
                            DeclKind::Unit(u)
                                if self.units[u].kind == crate::loader::UnitKind::Package =>
                            {
                                Region::Scope(self.units[u].scope)
                            }
                            DeclKind::Object(_) | DeclKind::Alias(Alias::Object { .. }) => {
                                return NameRes::Value;
                            }
                            DeclKind::Error => return NameRes::Error,
                            _ => {
                                return NameRes::Unsupported(
                                    n.span,
                                    "expanded names into this kind of region",
                                    "S5",
                                );
                            }
                        };
                        match region {
                            Region::Library(l) => match self.load_dep(
                                l,
                                crate::loader::UnitKey::Primary(key),
                                Some(n.span),
                            ) {
                                Ok(u) => {
                                    self.note_dep(u);
                                    NameRes::One(self.units[u].decl)
                                }
                                Err(_) => NameRes::Error,
                            },
                            Region::Scope(s) => match self.scopes[s].names.get(&key) {
                                Some(ds) if ds.len() == 1 && !self.is_overloadable(ds[0]) => {
                                    NameRes::One(ds[0])
                                }
                                Some(ds) if !ds.is_empty() => NameRes::Overloads(ds.clone()),
                                _ => NameRes::NotInRegion(s, suffix_span(suffix, n.span), key),
                            },
                        }
                    }
                    NameRes::Overloads(_) | NameRes::Value => NameRes::Value,
                    other => other,
                }
            }
            ast::NameKind::External(_) => NameRes::Unsupported(n.span, "external names", "S6"),
            ast::NameKind::Apply { .. } | ast::NameKind::Attribute { .. } => NameRes::Value,
            _ => NameRes::Unsupported(n.span, "this form of name", "S4"),
        }
    }

    /// Report why a name has no usable interpretation.
    pub(crate) fn report_name(&mut self, scope: ScopeId, r: NameRes) -> Reported {
        match r {
            NameRes::Missing(i) => self.undeclared(scope, i),
            NameRes::Conflict(i, c) => self.conflict(i, &c),
            NameRes::Unsupported(span, what, slice) => self.unsupported(span, what, slice),
            NameRes::NotInRegion(s, span, key) => self.not_in_region(s, span, key),
            NameRes::Error => Reported,
            NameRes::One(_) | NameRes::Overloads(_) | NameRes::Value => Reported,
        }
    }

    /// The subtype a type mark name denotes, without reporting.
    pub(crate) fn quiet_type_mark(&mut self, scope: ScopeId, n: &ast::Name) -> Option<SubtypeId> {
        match self.classify(scope, n) {
            NameRes::One(d) => match self.decls[self.entity(d)].kind {
                DeclKind::Type(t) | DeclKind::GenericType(t) => Some(self.types[t].first_subtype),
                DeclKind::Subtype(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// Bottom-up candidates of a name in an expression, memoized per name node.
    pub(crate) fn name_cands(&mut self, scope: ScopeId, n: &ast::Name) -> CandSet {
        let key = n as *const ast::Name as usize;
        if let Some(c) = self.name_memo.get(&key) {
            return c.clone();
        }
        let c = self.name_cands_uncached(scope, n);
        self.name_memo.insert(key, c.clone());
        c
    }

    fn name_cands_uncached(&mut self, scope: ScopeId, n: &ast::Name) -> CandSet {
        match &n.kind {
            ast::NameKind::Attribute { prefix, attr } => self.attr_cands(scope, prefix, *attr, &[]),
            ast::NameKind::Apply { prefix, args } => self.apply_cands(scope, prefix, args),
            ast::NameKind::Selected { prefix, suffix } => match self.classify(scope, n) {
                NameRes::Value => self.select_cands(scope, prefix, suffix),
                r => self.decl_cands(r),
            },
            _ => {
                let r = self.classify(scope, n);
                self.decl_cands(r)
            }
        }
    }

    fn decl_cands(&mut self, r: NameRes) -> CandSet {
        match r {
            NameRes::One(d) => match self.object_type(d) {
                Some(t) => CandSet::Types(vec![Interp::new(t, Via::Object)]),
                None => match self.decls[self.entity(d)].kind {
                    DeclKind::PhysUnit { ty, .. } => {
                        CandSet::Types(vec![Interp::new(ty, Via::Literal)])
                    }
                    DeclKind::Error => self.poison_cands(),
                    _ => CandSet::Types(Vec::new()),
                },
            },
            NameRes::Overloads(ds) => {
                CandSet::Types(ds.iter().filter_map(|&d| self.value_interp(d, 0)).collect())
            }
            NameRes::Error => self.poison_cands(),
            _ => CandSet::Types(Vec::new()),
        }
    }

    /// The types a name used as a prefix can have (objects and function results).
    pub(crate) fn prefix_types(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
    ) -> Result<Vec<TypeId>, ()> {
        match self.name_cands(scope, prefix) {
            CandSet::Types(is) => {
                if is.iter().any(|i| i.ty == self.poison.ty) {
                    return Err(());
                }
                let mut v: Vec<TypeId> = is.iter().map(|i| i.ty).collect();
                v.dedup();
                Ok(v)
            }
            CandSet::Class(_) => Ok(Vec::new()),
        }
    }

    /// `prefix.suffix` where the prefix is a value: a record element.
    fn select_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        suffix: &ast::Suffix,
    ) -> CandSet {
        let Some(key) = suffix_key(suffix) else {
            return CandSet::Types(Vec::new());
        };
        let Ok(ts) = self.prefix_types(scope, prefix) else {
            return self.poison_cands();
        };
        let mut out = Vec::new();
        for t in ts {
            if let (Some(i), TypeKind::Record { elems }) =
                (self.record_elem(t, key), &self.types[t].kind)
            {
                out.push(Interp::new(
                    self.base(elems[i as usize].subtype),
                    Via::Field(t, i),
                ));
            }
        }
        CandSet::Types(out)
    }

    /// Whether an association is a slice argument: a range, a subtype indication,
    /// a type mark or a range attribute.
    fn is_slice_arg(&mut self, scope: ScopeId, args: &[ast::AssocElem]) -> bool {
        match args {
            [
                ast::AssocElem {
                    formal: None,
                    actual,
                    ..
                },
            ] => match actual {
                ast::Actual::Range(_) | ast::Actual::Subtype(_) => true,
                ast::Actual::Expr(ast::Expr {
                    kind: ast::ExprKind::Name(n),
                    ..
                }) => match &n.kind {
                    ast::NameKind::Attribute { attr, .. } => {
                        matches!(attr.key.as_str(), "range" | "reverse_range")
                    }
                    _ => self.quiet_type_mark(scope, n).is_some(),
                },
                _ => false,
            },
            _ => false,
        }
    }

    /// Indexing or slicing a prefix of array type `t`.
    fn index_interp(
        &mut self,
        scope: ScopeId,
        t: TypeId,
        args: &[ast::AssocElem],
        slice: bool,
    ) -> Option<Interp> {
        let TypeKind::Array { indexes, elem } = &self.types[t].kind else {
            return None;
        };
        let (indexes, elem) = (indexes.clone(), *elem);
        if slice {
            return (indexes.len() == 1).then(|| Interp::new(t, Via::Slice(t)));
        }
        if args.len() != indexes.len() {
            return None;
        }
        let mut convs = 0;
        for (a, &ix) in args.iter().zip(indexes.iter()) {
            let (None, ast::Actual::Expr(x)) = (&a.formal, &a.actual) else {
                return None;
            };
            let c = self.cands(scope, x);
            convs += self.best_cost(&c, Expected::Type(self.base(ix)))?;
        }
        Some(Interp {
            convs,
            ..Interp::new(self.base(elem), Via::Index(t))
        })
    }

    /// Predefined attributes (16.2): of a type mark, or of an object or value.
    fn attr_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        args: &[ast::AssocElem],
    ) -> CandSet {
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return CandSet::Types(Vec::new());
        };
        let w = self.well_known.clone();
        use PredefAttr::*;
        let nargs = args.len();
        if matches!(a, SimpleName | PathName | InstanceName) {
            return match (w.string, nargs) {
                (Some(t), 0) => CandSet::Types(vec![Interp::new(t, Via::AttrOf(self.poison.ty))]),
                _ => CandSet::Types(Vec::new()),
            };
        }
        if let Some(st) = self.quiet_type_mark(scope, prefix) {
            let t = self.base(st);
            let is_array = matches!(self.types[t].kind, TypeKind::Array { .. });
            if is_array {
                return self.array_attr_cands(t, a, nargs, Via::Attr);
            }
            let (ty, convertible) = match (a, nargs) {
                (Left | Right | High | Low, 0) => (Some(t), false),
                (Ascending, 0) => (w.boolean, false),
                (Image, 1) => (w.string, false),
                (Value | Val | Succ | Pred | Leftof | Rightof, 1) => (Some(t), false),
                (Pos, 1) => (w.universal_integer, true),
                _ => (None, false),
            };
            return match ty {
                Some(ty) => CandSet::Types(vec![Interp {
                    convertible,
                    ..Interp::new(ty, Via::Attr)
                }]),
                None => CandSet::Types(Vec::new()),
            };
        }
        let Ok(ts) = self.prefix_types(scope, prefix) else {
            return self.poison_cands();
        };
        let mut out = Vec::new();
        for t in ts {
            let via = Via::AttrOf(t);
            if matches!(self.types[t].kind, TypeKind::Array { .. })
                && let CandSet::Types(v) = self.array_attr_cands(t, a, nargs, via)
                && !v.is_empty()
            {
                out.extend(v);
                continue;
            }
            let ty = match (a, nargs) {
                (Event | Active | Stable | Quiet | Driving, _) if nargs <= 1 => w.boolean,
                (LastEvent | LastActive, 0) => w.time,
                (LastValue | DrivingValue, 0) | (Delayed, 0 | 1) => Some(t),
                (Transaction, 0) => w.bit,
                (Image, 0) => w.string,
                _ => None,
            };
            if let Some(ty) = ty {
                out.push(Interp::new(ty, via));
            }
        }
        CandSet::Types(out)
    }

    /// Array attributes with an optional dimension argument.
    fn array_attr_cands(&mut self, t: TypeId, a: PredefAttr, nargs: usize, via: Via) -> CandSet {
        use PredefAttr::*;
        let TypeKind::Array { indexes, .. } = &self.types[t].kind else {
            return CandSet::Types(Vec::new());
        };
        let index = self.base(indexes[0]);
        let w = &self.well_known;
        let (ty, convertible) = match (a, nargs) {
            (Left | Right | High | Low, 0 | 1) => (Some(index), false),
            (Length, 0 | 1) => (w.universal_integer, true),
            (Ascending, 0 | 1) => (w.boolean, false),
            _ => (None, false),
        };
        match ty {
            Some(ty) => CandSet::Types(vec![Interp {
                convertible,
                ..Interp::new(ty, via)
            }]),
            None => CandSet::Types(Vec::new()),
        }
    }

    fn apply_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        args: &[ast::AssocElem],
    ) -> CandSet {
        if let ast::NameKind::Attribute { prefix: p, attr } = &prefix.kind {
            return self.attr_cands(scope, p, *attr, args);
        }
        if let Some(st) = self.quiet_type_mark(scope, prefix) {
            // Type conversion (9.3.6).
            return CandSet::Types(vec![Interp::new(self.base(st), Via::Other)]);
        }
        let slice = self.is_slice_arg(scope, args);
        let mut out = Vec::new();
        match self.classify(scope, prefix) {
            NameRes::Overloads(ds) => {
                for &d in &ds {
                    if !slice && let Some(i) = self.call_interp(scope, d, args) {
                        out.push(i);
                    }
                }
                // A parameterless function whose result is indexed or sliced (8.3).
                for d in ds {
                    if let Some(i) = self.value_interp(d, 0)
                        && let Some(x) = self.index_interp(scope, i.ty, args, slice)
                    {
                        out.push(x);
                    }
                }
            }
            NameRes::Error => return self.poison_cands(),
            NameRes::One(_) | NameRes::Value => {
                let Ok(ts) = self.prefix_types(scope, prefix) else {
                    return self.poison_cands();
                };
                for t in ts {
                    if let Some(x) = self.index_interp(scope, t, args, slice) {
                        out.push(x);
                    }
                }
            }
            _ => {}
        }
        CandSet::Types(out)
    }

    pub(crate) fn object_type(&self, d: DeclId) -> Option<TypeId> {
        match &self.decls[d].kind {
            DeclKind::Object(o) => Some(self.base(o.subtype)),
            DeclKind::Alias(Alias::Object { target, subtype }) => {
                Some(self.base(subtype.unwrap_or(self.exprs[*target].subtype)))
            }
            _ => None,
        }
    }

    fn call_interp(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        args: &[ast::AssocElem],
    ) -> Option<Interp> {
        let e = self.entity(d);
        let s = self.decls[e].kind.subprogram()?;
        let ret = self.base(s.ret?);
        let formals: Vec<(Symbol, TypeId, bool)> = s
            .params
            .iter()
            .map(|p| (p.name.key, self.base(p.subtype), p.default.is_some()))
            .collect();
        let map = assoc_map(&formals.iter().map(|f| f.0).collect::<Vec<_>>(), args)?;
        let mut convs = 0;
        for (i, (_, ft, has_default)) in formals.iter().enumerate() {
            match map[i] {
                Some(a) => {
                    let ast::Actual::Expr(x) = &args[a].actual else {
                        return None;
                    };
                    let c = self.cands(scope, x);
                    convs += self.best_cost(&c, Expected::Type(*ft))?;
                }
                None if *has_default => {}
                None => return None,
            }
        }
        Some(Interp {
            convs,
            ..Interp::new(ret, Via::Call(d))
        })
    }

    // -- top-down ---------------------------------------------------------------

    /// Resolve a name used as a prefix to the interpretation of type `t`.
    pub(crate) fn resolve_prefix(&mut self, scope: ScopeId, n: &ast::Name, t: TypeId) -> ExprId {
        let CandSet::Types(is) = self.name_cands(scope, n) else {
            return self.poison_expr();
        };
        let mut best: Vec<Interp> = Vec::new();
        for i in is {
            if (i.ty == t || i.ty == self.poison.ty) && !best.iter().any(|b| b.via == i.via) {
                best.push(i);
            }
        }
        best.sort_by_key(|i| i.convs);
        match best.as_slice() {
            [] => self.poison_expr(),
            [i] => self.build_name(scope, n, *i, n.span),
            [i, j, ..] if i.convs < j.convs => self.build_name(scope, n, *i, n.span),
            _ => {
                self.report(
                    Diagnostic::error(diag::AMBIGUOUS, "this prefix is ambiguous")
                        .with_primary(
                            n.span,
                            "several interpretations of this name have the same type",
                        )
                        .with_help("use a selected name or qualify the value"),
                );
                self.poison_expr()
            }
        }
    }

    /// Build the HIR of name `n` for the chosen interpretation.
    pub(crate) fn build_name(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
        i: Interp,
        span: Span,
    ) -> ExprId {
        match (&n.kind, i.via) {
            (ast::NameKind::Attribute { prefix, attr }, _) => {
                self.build_attr(scope, prefix, *attr, &[], i, span)
            }
            (ast::NameKind::Apply { prefix, args }, _) => {
                if let ast::NameKind::Attribute { prefix: p, attr } = &prefix.kind {
                    return self.build_attr(scope, p, *attr, args, i, span);
                }
                if let Some(st) = self.quiet_type_mark(scope, prefix) {
                    return self.build_conversion(scope, st, args, span);
                }
                match i.via {
                    Via::Call(d) => self.build_call(scope, d, args, span, None),
                    Via::Index(t) => self.build_index(scope, prefix, t, args, span),
                    Via::Slice(t) => self.build_slice(scope, prefix, t, args, span),
                    _ => self.poison_expr(),
                }
            }
            (ast::NameKind::Selected { prefix, .. }, Via::Field(t, elem)) => {
                let p = self.resolve_prefix(scope, prefix, t);
                let subtype = match &self.types[t].kind {
                    TypeKind::Record { elems } => elems[elem as usize].subtype,
                    _ => self.poison.subtype,
                };
                let stat = self.exprs[p].stat;
                self.add_expr(Expr {
                    kind: ExprKind::Field { prefix: p, elem },
                    subtype,
                    span,
                    stat,
                    value: None,
                })
            }
            (_, Via::Call(d)) => self.build_call(scope, d, &[], span, None),
            (_, Via::EnumLit(d)) => {
                let DeclKind::EnumLit { ty, pos } = self.decls[d].kind else {
                    return self.poison_expr();
                };
                let st = self.types[ty].first_subtype;
                self.lit_expr(ConstValue::Enum(pos), st, span)
            }
            (_, Via::Literal) => match self.classify(scope, n) {
                NameRes::One(d) => match self.decls[self.entity(d)].kind {
                    DeclKind::PhysUnit { ty, index } => {
                        let scale = match &self.types[ty].kind {
                            TypeKind::Physical { units } => units[index as usize].scale,
                            _ => return self.poison_expr(),
                        };
                        let st = self.types[ty].first_subtype;
                        self.lit_expr(ConstValue::Phys(scale), st, span)
                    }
                    _ => self.poison_expr(),
                },
                _ => self.poison_expr(),
            },
            (_, Via::Object) => match self.classify(scope, n) {
                NameRes::One(d) => self.object_expr(d, span),
                _ => self.poison_expr(),
            },
            _ => self.poison_expr(),
        }
    }

    fn build_index(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        t: TypeId,
        args: &[ast::AssocElem],
        span: Span,
    ) -> ExprId {
        let p = self.resolve_prefix(scope, prefix, t);
        let TypeKind::Array { indexes, elem } = &self.types[t].kind else {
            return self.poison_expr();
        };
        let (indexes, elem) = (indexes.clone(), *elem);
        let mut stat = self.exprs[p].stat;
        let mut ids = Vec::new();
        for (a, ix) in args.iter().zip(indexes.iter()) {
            let ast::Actual::Expr(x) = &a.actual else {
                return self.poison_expr();
            };
            let id = self.resolve(scope, x, Expected::Type(self.base(*ix)));
            stat = stat.min(self.exprs[id].stat);
            ids.push(id);
        }
        // The element subtype of the prefix's own subtype (2008 element constraints).
        let pst = self.exprs[p].subtype;
        let subtype = self.element_subtype(pst).unwrap_or(elem);
        self.add_expr(Expr {
            kind: ExprKind::Index {
                prefix: p,
                indices: ids.into(),
            },
            subtype,
            span,
            stat,
            value: None,
        })
    }

    fn build_slice(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        t: TypeId,
        args: &[ast::AssocElem],
        span: Span,
    ) -> ExprId {
        let p = self.resolve_prefix(scope, prefix, t);
        let TypeKind::Array { indexes, .. } = &self.types[t].kind else {
            return self.poison_expr();
        };
        let index = self.base(indexes[0]);
        let range = match &args[0].actual {
            ast::Actual::Range(r) => self.range(scope, r, index),
            ast::Actual::Subtype(si) => {
                self.subtype_indication(scope, si)
                    .map(|st| crate::types::RangeC::Attr {
                        prefix: AttrPrefix::Subtype(st),
                        reverse: false,
                        dim: 1,
                    })
            }
            ast::Actual::Expr(ast::Expr {
                kind: ast::ExprKind::Name(n),
                ..
            }) => self.range(scope, &ast::Range::Name(clone_name_shallow(n)), index),
            _ => return self.poison_expr(),
        };
        let Ok(range) = range else {
            return self.poison_expr();
        };
        let pst = self.exprs[p].subtype;
        let elem_c = match &self.subtypes[pst].constraint {
            crate::types::Constraint::Array { elem, .. } => elem.clone(),
            _ => Box::new(crate::types::Constraint::None),
        };
        let stat = self.exprs[p].stat.min(Staticness::Globally);
        let subtype = self.subtypes.push(crate::types::SubtypeDef {
            base: t,
            name: None,
            parent: Some(pst),
            constraint: crate::types::Constraint::Array {
                dims: Some(Box::new([range.clone()])),
                elem: elem_c,
            },
            resolution: self.subtypes[pst].resolution.clone(),
            span,
        });
        self.add_expr(Expr {
            kind: ExprKind::Slice { prefix: p, range },
            subtype,
            span,
            stat,
            value: None,
        })
    }

    /// The element subtype of an array subtype, with its element constraint applied.
    pub(crate) fn element_subtype(&mut self, st: SubtypeId) -> Option<SubtypeId> {
        let TypeKind::Array { elem, .. } = &self.types[self.base(st)].kind else {
            return None;
        };
        let elem = *elem;
        match &self.subtypes[st].constraint {
            crate::types::Constraint::Array { elem: c, .. }
                if !matches!(**c, crate::types::Constraint::None) =>
            {
                let c = (**c).clone();
                let base = self.base(elem);
                Some(self.subtypes.push(crate::types::SubtypeDef {
                    base,
                    name: None,
                    parent: Some(elem),
                    constraint: c,
                    resolution: self.subtypes[elem].resolution.clone(),
                    span: Span::default(),
                }))
            }
            _ => Some(elem),
        }
    }

    pub(crate) fn lit_expr(&mut self, v: ConstValue, st: SubtypeId, span: Span) -> ExprId {
        self.const_expr(v, st, span)
    }

    pub(crate) fn poison_expr(&self) -> ExprId {
        self.poison.expr
    }

    /// A reference to an object; constants carry their folded value (9.4.1).
    pub(crate) fn object_ref(&mut self, d: DeclId, span: Span) -> ExprId {
        self.object_expr(d, span)
    }

    fn object_expr(&mut self, d: DeclId, span: Span) -> ExprId {
        let (subtype, stat, value) = match &self.decls[d].kind {
            DeclKind::Object(o) => {
                let (stat, value) = match (o.class, &o.role, o.default) {
                    (crate::decl::ObjClass::Constant, crate::decl::ObjRole::Plain, Some(init)) => {
                        let e = &self.exprs[init];
                        (
                            e.stat,
                            if e.stat == Staticness::Locally {
                                e.value
                            } else {
                                None
                            },
                        )
                    }
                    // Loop parameters are not static; generate parameters are globally static (8.1, NVC #856).
                    (_, crate::decl::ObjRole::LoopParam, _) => (Staticness::None, None),
                    (crate::decl::ObjClass::Constant, _, _) => (Staticness::Globally, None),
                    _ => (Staticness::None, None),
                };
                (o.subtype, stat, value)
            }
            DeclKind::Alias(Alias::Object { target, subtype }) => {
                let e = &self.exprs[*target];
                (subtype.unwrap_or(e.subtype), e.stat, e.value)
            }
            _ => return self.poison_expr(),
        };
        self.add_expr(Expr {
            kind: ExprKind::Obj(d),
            subtype,
            span,
            stat,
            value,
        })
    }

    /// A function call with named or positional association; defaults are `Actual::Default`.
    pub(crate) fn build_call(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        args: &[ast::AssocElem],
        span: Span,
        method_of: Option<ExprId>,
    ) -> ExprId {
        let e = self.entity(d);
        let Some(s) = self.decls[e].kind.subprogram() else {
            return self.poison_expr();
        };
        let formals: Vec<(Symbol, TypeId)> = s
            .params
            .iter()
            .map(|p| (p.name.key, self.base(p.subtype)))
            .collect();
        let ret = s.ret.unwrap_or(self.poison.subtype);
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        let Some(map) = assoc_map(&formals.iter().map(|f| f.0).collect::<Vec<_>>(), args) else {
            return self.poison_expr();
        };
        let mut hargs = Vec::with_capacity(formals.len());
        let mut stat = if implicit {
            Staticness::Locally
        } else {
            Staticness::None
        };
        for (k, (_, ft)) in formals.iter().enumerate() {
            let actual = match map[k] {
                Some(a) => match &args[a].actual {
                    ast::Actual::Expr(x) => {
                        let id = self.resolve(scope, x, Expected::Type(*ft));
                        stat = stat.min(self.exprs[id].stat);
                        Actual::Expr(id)
                    }
                    ast::Actual::Open(_) => Actual::Open,
                    other => {
                        let sp = actual_span(other).unwrap_or(span);
                        self.unsupported(sp, "this kind of actual", "S2");
                        return self.poison_expr();
                    }
                },
                None => Actual::Default,
            };
            hargs.push(Arg {
                formal: k as u16,
                actual,
            });
        }
        let id = self.add_expr(Expr {
            kind: ExprKind::Call {
                callee: e,
                args: hargs.into(),
                method_of,
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

    fn build_conversion(
        &mut self,
        scope: ScopeId,
        st: SubtypeId,
        args: &[ast::AssocElem],
        span: Span,
    ) -> ExprId {
        let [
            ast::AssocElem {
                formal: None,
                actual: ast::Actual::Expr(x),
                ..
            },
        ] = args
        else {
            return Expr::poison_of(self, span, "a type conversion takes one operand");
        };
        let operand = self.resolve(scope, x, Expected::Any);
        let stat = self.exprs[operand].stat;
        let id = self.add_expr(Expr {
            kind: ExprKind::Conv { to: st, operand },
            subtype: st,
            span,
            stat,
            value: None,
        });
        if stat == Staticness::Locally {
            self.fold_into(id);
        }
        id
    }

    /// Predefined attributes: of a type mark (`T'high`, `T'pos(x)`, ...) or of
    /// an object or value (`a'length`, `s'event`, ...).
    fn build_attr(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        args: &[ast::AssocElem],
        i: Interp,
        span: Span,
    ) -> ExprId {
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return self.poison_expr();
        };
        let (aprefix, mut stat, arg_t) = match i.via {
            Via::AttrOf(t) if t == self.poison.ty => {
                // 'simple_name, 'path_name, 'instance_name of a named entity.
                let d = match self.classify(scope, prefix) {
                    NameRes::One(d) => d,
                    NameRes::Overloads(ds) => ds[0],
                    r => return Expr::poison_reported(self, scope, r),
                };
                let stat = if a == PredefAttr::SimpleName {
                    Staticness::Locally
                } else {
                    Staticness::Globally
                };
                (AttrPrefix::Entity(d), stat, None)
            }
            Via::AttrOf(t) => {
                if matches!(
                    a,
                    PredefAttr::Delayed
                        | PredefAttr::Stable
                        | PredefAttr::Quiet
                        | PredefAttr::Transaction
                ) && self.in_subprogram(scope)
                {
                    self.report(
                        Diagnostic::error(diag::SIGNAL_ATTR_IN_SUBPROGRAM, format!("`'{}` is not allowed in a subprogram", attr.text))
                            .with_primary(attr.span, "creates an implicit signal")
                            .with_note("LRM 16.2.4: implicit signal attributes are only allowed outside subprograms"),
                    );
                    return self.poison_expr();
                }
                let p = self.resolve_prefix(scope, prefix, t);
                let stat = match (a, &self.types[t].kind) {
                    (
                        PredefAttr::Left
                        | PredefAttr::Right
                        | PredefAttr::High
                        | PredefAttr::Low
                        | PredefAttr::Length
                        | PredefAttr::Ascending,
                        TypeKind::Array { .. },
                    ) => {
                        if self.constrainedness(self.exprs[p].subtype)
                            == crate::types::Constrainedness::Full
                        {
                            Staticness::Globally
                        } else {
                            Staticness::None
                        }
                    }
                    _ => Staticness::None,
                };
                (AttrPrefix::Expr(p), stat, Some(t))
            }
            _ => {
                let Some(st) = self.quiet_type_mark(scope, prefix) else {
                    return self.poison_expr();
                };
                (
                    AttrPrefix::Subtype(st),
                    self.subtype_staticness(st),
                    Some(self.base(st)),
                )
            }
        };
        let t = arg_t.unwrap_or(self.poison.ty);
        let is_array = matches!(self.types[t].kind, TypeKind::Array { .. });
        let mut hargs = Vec::new();
        for arg in args {
            let ast::Actual::Expr(x) = &arg.actual else {
                return self.poison_expr();
            };
            let id = match a {
                PredefAttr::Value => self.resolve(
                    scope,
                    x,
                    Expected::Type(self.well_known.string.unwrap_or(self.poison.ty)),
                ),
                PredefAttr::Val => self.resolve(scope, x, Expected::Any),
                PredefAttr::Delayed | PredefAttr::Stable | PredefAttr::Quiet => self.resolve(
                    scope,
                    x,
                    Expected::Type(self.well_known.time.unwrap_or(self.poison.ty)),
                ),
                _ if is_array => self.static_expr(scope, x, Expected::Any),
                _ => self.resolve(scope, x, Expected::Type(t)),
            };
            stat = stat.min(self.exprs[id].stat);
            hargs.push(id);
        }
        let subtype = self.types[i.ty].first_subtype;
        let id = self.add_expr(Expr {
            kind: ExprKind::Attr {
                attr: a,
                prefix: aprefix,
                args: hargs.into(),
            },
            subtype,
            span,
            stat,
            value: None,
        });
        let array_st = match aprefix {
            AttrPrefix::Expr(p) if is_array => Some(self.exprs[p].subtype),
            AttrPrefix::Subtype(st) if is_array => Some(st),
            _ => None,
        };
        if let Some(st) = array_st {
            self.fold_array_attr(id, st, a);
        } else if stat == Staticness::Locally {
            self.fold_into(id);
        }
        id
    }

    /// `'left`, `'length`, ... of a static array subtype: folded even when the
    /// prefix is an object (its subtype decides, 16.2.3).
    fn fold_array_attr(&mut self, id: ExprId, st: SubtypeId, a: PredefAttr) {
        let dim = match &self.exprs[id].kind {
            ExprKind::Attr { args, .. } => match args.first() {
                Some(&x) => match self.value(x).and_then(|v| v.as_int()) {
                    Some(d) if d >= 1 => d as usize - 1,
                    Some(_) => return,
                    None => return,
                },
                None => 0,
            },
            _ => return,
        };
        let Some(r) = crate::expr::aggregate::subtype_range(self, st, dim) else {
            return;
        };
        let ix_kind = match &self.types[self.base(st)].kind {
            TypeKind::Array { indexes, .. } => indexes
                .get(dim)
                .map(|&i| matches!(self.types[self.base(i)].kind, TypeKind::Enum { .. })),
            _ => None,
        };
        let Some(is_enum) = ix_kind else { return };
        let mk = |v: i64| {
            if is_enum {
                ConstValue::Enum(v as u32)
            } else {
                ConstValue::Int(v)
            }
        };
        let v = match a {
            PredefAttr::Left => mk(r.left),
            PredefAttr::Right => mk(r.right),
            PredefAttr::High => mk(r.high()),
            PredefAttr::Low => mk(r.low()),
            PredefAttr::Length => match i64::try_from(r.len()) {
                Ok(n) => ConstValue::Int(n),
                Err(_) => return,
            },
            PredefAttr::Ascending => ConstValue::Enum(u32::from(r.dir == Direction::To)),
            _ => return,
        };
        let c = self.add_const(v);
        self.exprs[id].value = Some(c);
        if self.exprs[id].stat == Staticness::None {
            self.exprs[id].stat = Staticness::Globally;
        }
    }

    /// Whether `scope` is inside a subprogram body or declaration.
    pub(crate) fn in_subprogram(&self, scope: ScopeId) -> bool {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            match self.scopes[s].kind {
                crate::scope::ScopeKind::Subprogram => return true,
                crate::scope::ScopeKind::Process
                | crate::scope::ScopeKind::Architecture
                | crate::scope::ScopeKind::Block => return false,
                _ => {}
            }
            cur = self.scopes[s].parent;
        }
        false
    }

    /// A subtype is locally static when its range bounds are (9.4.2, scalar case).
    pub(crate) fn subtype_staticness(&self, st: SubtypeId) -> Staticness {
        match self.scalar_range(st) {
            Some(crate::types::RangeC::Explicit { left, right, .. }) => {
                self.exprs[left].stat.min(self.exprs[right].stat)
            }
            Some(crate::types::RangeC::Attr { .. }) => Staticness::Globally,
            None => Staticness::Locally,
        }
    }

    /// `high` bound helper for attribute folding.
    pub(crate) fn bound(&self, st: SubtypeId, which: PredefAttr) -> Option<ConstValue> {
        let (l, dir, r) = self.static_bounds(st)?;
        Some(match (which, dir) {
            (PredefAttr::Left, _) => l,
            (PredefAttr::Right, _) => r,
            (PredefAttr::High, Direction::To) | (PredefAttr::Low, Direction::Downto) => r,
            (PredefAttr::Low, Direction::To) | (PredefAttr::High, Direction::Downto) => l,
            _ => return None,
        })
    }
}

impl Expr {
    fn poison_reported(db: &mut Db, scope: ScopeId, r: NameRes) -> ExprId {
        db.report_name(scope, r);
        db.poison.expr
    }

    fn poison_of(db: &mut Db, span: Span, msg: &str) -> ExprId {
        db.report(
            Diagnostic::error(diag::BAD_CONVERSION, msg.to_string()).with_primary(span, "here"),
        );
        db.poison.expr
    }
}

fn actual_span(a: &ast::Actual) -> Option<Span> {
    match a {
        ast::Actual::Expr(e) | ast::Actual::Inertial(e) => Some(e.span),
        ast::Actual::Open(s) => Some(*s),
        ast::Actual::Subtype(si) => Some(si.span),
        ast::Actual::Range(_) => None,
    }
}

/// Map formals (by key) to association indexes: positional first, then named.
/// `None` if the association does not fit (too many, unknown or repeated name).
// ponytail: named association by simple formal name only; sub-element and conversion
// formals (6.5.7.1) come with S5.
pub(crate) fn assoc_map(formals: &[Symbol], args: &[ast::AssocElem]) -> Option<Vec<Option<usize>>> {
    let mut map = vec![None; formals.len()];
    let mut named = false;
    for (k, a) in args.iter().enumerate() {
        match &a.formal {
            None => {
                if named || k >= formals.len() {
                    return None;
                }
                map[k] = Some(k);
            }
            Some(f) => {
                named = true;
                let ast::NameKind::Simple(i) = &f.kind else {
                    return None;
                };
                let pos = formals.iter().position(|&x| x == i.key)?;
                if map[pos].is_some() {
                    return None;
                }
                map[pos] = Some(k);
            }
        }
    }
    Some(map)
}

/// A copy of a simple, selected or attribute name for re-analysis as a range.
fn clone_name_shallow(n: &ast::Name) -> ast::Name {
    fn go(n: &ast::Name) -> ast::Name {
        let kind = match &n.kind {
            ast::NameKind::Simple(i) => ast::NameKind::Simple(*i),
            ast::NameKind::Operator(o) => ast::NameKind::Operator(*o),
            ast::NameKind::Character(c) => ast::NameKind::Character(*c),
            ast::NameKind::Selected { prefix, suffix } => ast::NameKind::Selected {
                prefix: Box::new(go(prefix)),
                suffix: match suffix {
                    ast::Suffix::Ident(i) => ast::Suffix::Ident(*i),
                    ast::Suffix::Char(c) => ast::Suffix::Char(*c),
                    ast::Suffix::Operator(o, s) => ast::Suffix::Operator(*o, *s),
                    ast::Suffix::All(s) => ast::Suffix::All(*s),
                },
            },
            ast::NameKind::Attribute { prefix, attr } => ast::NameKind::Attribute {
                prefix: Box::new(go(prefix)),
                attr: *attr,
            },
            // Other forms never denote a range; keep the span for the diagnostic.
            _ => ast::NameKind::Simple(Ident {
                text: Symbol::intern("<name>"),
                key: Symbol::intern("<name>"),
                span: n.span,
            }),
        };
        ast::Name { kind, span: n.span }
    }
    go(n)
}
