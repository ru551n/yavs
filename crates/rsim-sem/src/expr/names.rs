//! Name interpretation: simple and expanded names, `Apply` (calls, conversions,
//! attribute functions), predefined attributes (sem.md 1.5, 3.3).
//!
//! Both passes go through [`Db::classify`], which resolves the non-overloaded
//! prefix chain of a name without reporting; the top-down pass reports.

use rsim_diag::{Diagnostic, Span};
use rsim_syntax::Symbol;
use rsim_syntax::ast::{self, Ident};

use super::{CandSet, Expected, Interp, Via};
use crate::attr::{self, AttrInfo, Kind, PredefAttr, PrefixForm};
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
    /// An uninstantiated generic package used as a prefix (`E1505`, generic.rs).
    Uninstantiated(DeclId, Span),
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
                    // `p.all` (alloc.rs): a value when the prefix is one.
                    return match self.classify(scope, prefix) {
                        NameRes::One(_) | NameRes::Overloads(_) | NameRes::Value => NameRes::Value,
                        other => other,
                    };
                };
                match self.classify(scope, prefix) {
                    NameRes::One(d) => {
                        let region = match self.decls[self.entity(d)].kind {
                            DeclKind::Library(l) => Region::Library(l),
                            DeclKind::Unit(_)
                            | DeclKind::PackageInst(_)
                            | DeclKind::GenericPackage(_)
                                if !matches!(
                                    self.package_region(d),
                                    crate::generic::PkgRegion::No
                                ) =>
                            {
                                match self.package_region(d) {
                                    crate::generic::PkgRegion::Scope(s) => Region::Scope(s),
                                    _ => {
                                        return NameRes::Uninstantiated(
                                            self.entity(d),
                                            crate::generic::last_span(prefix),
                                        );
                                    }
                                }
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
            NameRes::Uninstantiated(d, span) => self.uninstantiated(d, span),
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
        let Ok(ts) = self.prefix_types(scope, prefix) else {
            return self.poison_cands();
        };
        let Some(key) = suffix_key(suffix) else {
            // `.all`
            let is = ts
                .into_iter()
                .filter_map(|t| Some(Interp::new(self.base(self.designated(t)?), Via::Deref(t))))
                .collect();
            return CandSet::Types(is);
        };
        let mut out = Vec::new();
        for t in self.with_designated(ts) {
            for d in self.method_decls(t, key) {
                if let Some(i) = self.value_interp(d, 0) {
                    out.push(Interp {
                        via: Via::Method(d, t),
                        ..i
                    });
                }
            }
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

    /// What an attribute prefix can denote, without reporting (16.1): each
    /// form with where its subtype comes from. `Err`: the prefix name itself
    /// does not resolve.
    fn attr_prefixes(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
    ) -> Result<Vec<(attr::Prefix, PSrc)>, NameRes> {
        let entity = |d| {
            (
                attr::Prefix {
                    form: PrefixForm::Entity,
                    subtype: None,
                    fully_constrained: false,
                },
                PSrc::Entity(d),
            )
        };
        if let Some(st) = self.quiet_type_mark(scope, prefix) {
            return Ok(vec![(
                attr::Prefix {
                    form: PrefixForm::TypeMark,
                    subtype: Some(st),
                    fully_constrained: self.constrainedness(st)
                        == crate::types::Constrainedness::Full,
                },
                PSrc::Type(st),
            )]);
        }
        let inner = match &prefix.kind {
            ast::NameKind::Signature { prefix, .. } => prefix.as_ref(),
            _ => prefix,
        };
        match self.classify(scope, inner) {
            NameRes::One(d) => {
                let e = self.entity(d);
                if let Some(t) = self.object_type(d) {
                    let subtype = self.object_subtype(d);
                    let mut out = vec![(
                        attr::Prefix {
                            form: PrefixForm::Object {
                                signal: self.decl_is_signal(d),
                            },
                            subtype,
                            fully_constrained: subtype.is_some_and(|st| {
                                self.constrainedness(st) == crate::types::Constrainedness::Full
                            }),
                        },
                        PSrc::Object(d, t),
                    )];
                    // An access object is appropriate for its designated type (8.1).
                    if let Some(des) = self.designated(t) {
                        out.push((
                            attr::Prefix {
                                form: PrefixForm::Object { signal: false },
                                subtype: Some(des),
                                fully_constrained: false,
                            },
                            PSrc::Value(self.base(des)),
                        ));
                    }
                    return Ok(out);
                }
                if matches!(self.decls[e].kind, DeclKind::View(_)) {
                    return Ok(vec![(
                        attr::Prefix {
                            form: PrefixForm::View,
                            subtype: None,
                            fully_constrained: false,
                        },
                        PSrc::Entity(d),
                    )]);
                }
                Ok(vec![entity(d)])
            }
            NameRes::Overloads(ds) => {
                // The subprogram or literal as a named entity first; then the
                // result of a parameterless function call (8.1 "appropriate for").
                let mut out: Vec<(attr::Prefix, PSrc)> = ds.iter().map(|&d| entity(d)).collect();
                for d in ds {
                    if let Some(i) = self.value_interp(d, 0)
                        && !out.iter().any(|(_, s)| *s == PSrc::Value(i.ty))
                    {
                        out.push(self.value_prefix(i.ty, false));
                    }
                }
                Ok(out)
            }
            NameRes::Value => {
                let Ok(ts) = self.prefix_types(scope, prefix) else {
                    return Err(NameRes::Error);
                };
                let signal = self.ast_root_signal(scope, prefix);
                Ok(self
                    .with_designated(ts)
                    .into_iter()
                    .map(|t| self.value_prefix(t, signal))
                    .collect())
            }
            r => Err(r),
        }
    }

    fn value_prefix(&self, t: TypeId, signal: bool) -> (attr::Prefix, PSrc) {
        (
            attr::Prefix {
                form: PrefixForm::Object { signal },
                subtype: Some(self.types[t].base_subtype),
                fully_constrained: false,
            },
            PSrc::Value(t),
        )
    }

    /// The declared subtype of an object or object alias.
    fn object_subtype(&self, d: DeclId) -> Option<SubtypeId> {
        match &self.decls[d].kind {
            DeclKind::Object(o) => Some(o.subtype),
            DeclKind::Alias(Alias::Object { target, subtype }) => {
                Some(subtype.unwrap_or(self.exprs[*target].subtype))
            }
            _ => None,
        }
    }

    /// Is `d` a signal (ports, implicit signals and signal aliases included)?
    pub(crate) fn decl_is_signal(&self, d: DeclId) -> bool {
        match &self.decls[d].kind {
            DeclKind::Object(o) => o.class == crate::decl::ObjClass::Signal,
            DeclKind::Alias(Alias::Object { target, .. }) => self.expr_is_signal(*target),
            _ => false,
        }
    }

    /// Is `e` a name of a signal or of a part of one?
    pub(crate) fn expr_is_signal(&self, e: ExprId) -> bool {
        match &self.exprs[e].kind {
            ExprKind::Obj(d) => self.decl_is_signal(*d),
            ExprKind::External(x) => {
                self.externals[*x].class == rsim_syntax::ast::ExternalClass::Signal
            }
            ExprKind::Index { prefix, .. }
            | ExprKind::Slice { prefix, .. }
            | ExprKind::Field { prefix, .. } => self.expr_is_signal(*prefix),
            _ => false,
        }
    }

    /// Whether the object a selected or indexed name starts from is a signal.
    fn ast_root_signal(&mut self, scope: ScopeId, n: &ast::Name) -> bool {
        match self.classify(scope, n) {
            NameRes::One(d) => self.decl_is_signal(d),
            NameRes::Value => match &n.kind {
                ast::NameKind::Selected { prefix, .. } | ast::NameKind::Apply { prefix, .. } => {
                    self.ast_root_signal(scope, prefix)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Attributes (16.2 predefined through the `attr::TABLE`, 6.7 user-defined).
    fn attr_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        args: &[ast::AssocElem],
    ) -> CandSet {
        // No interpretation: the top-down pass reports why (`report_attr`).
        let ps = match self.attr_prefixes(scope, prefix) {
            Ok(ps) => ps,
            Err(NameRes::Error) => return self.poison_cands(),
            Err(_) => return CandSet::Types(Vec::new()),
        };
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return match self.user_attr_type(&ps, attr.key) {
                Some(t) if args.is_empty() => CandSet::Types(vec![Interp::new(t, Via::Attr)]),
                _ => CandSet::Types(Vec::new()),
            };
        };
        let info = a.info();
        let dim = literal_dim(args);
        let mut out = Vec::new();
        for (p, src) in &ps {
            if !attr::applies(self, info, p)
                || !matches!(info.kind, Kind::Value | Kind::Function | Kind::Signal)
                || !arity_ok(info, p, args.len())
            {
                continue;
            }
            let st = if info.result == attr::Result_::Mirror {
                self.mirror_subtype(p.form == PrefixForm::TypeMark)
            } else {
                attr::result_subtype(self, info, p, dim)
            };
            let Some(st) = st else {
                continue;
            };
            let via = match *src {
                _ if info
                    .prefixes
                    .iter()
                    .all(|r| r.form == attr::Form::NamedEntity) =>
                {
                    Via::AttrOf(self.poison.ty)
                }
                PSrc::Type(_) => Via::Attr,
                PSrc::Value(t) | PSrc::Object(_, t) => Via::AttrOf(t),
                PSrc::Entity(_) => Via::AttrOf(self.poison.ty),
            };
            out.push(Interp {
                convertible: info.result == attr::Result_::UniversalInteger,
                ..Interp::new(self.base(st), via)
            });
        }
        CandSet::Types(out)
    }

    /// The type of user-defined attribute `key` of the named entity a prefix
    /// denotes, if it has a value for it (7.2). Any overload with a value counts.
    fn user_attr_type(&self, ps: &[(attr::Prefix, PSrc)], key: Symbol) -> Option<TypeId> {
        ps.iter().find_map(|(_, src)| {
            let (PSrc::Entity(d) | PSrc::Object(d, _)) = *src else {
                return None;
            };
            let v = attr::user_value(self, self.entity(d), key)?;
            let st = attr::spec_subtype(self, self.attr_values[v].attr)?;
            Some(self.base(st))
        })
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
                if !slice {
                    out.extend(self.method_call_cands(scope, prefix, args));
                }
                let Ok(ts) = self.prefix_types(scope, prefix) else {
                    return self.poison_cands();
                };
                for t in self.with_designated(ts) {
                    if let Some(x) = self.index_interp(scope, t, args, slice) {
                        out.push(x);
                    }
                }
            }
            _ => {}
        }
        CandSet::Types(out)
    }

    /// `x.m(args)` with `x` a protected object or an access to one: the
    /// methods `m` that accept `args`.
    fn method_call_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        args: &[ast::AssocElem],
    ) -> Vec<Interp> {
        let ast::NameKind::Selected {
            prefix: obj,
            suffix,
        } = &prefix.kind
        else {
            return Vec::new();
        };
        let (Some(key), Ok(ts)) = (suffix_key(suffix), self.prefix_types(scope, obj)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for t in ts {
            for d in self.method_decls(t, key) {
                if let Some(i) = self.call_interp(scope, d, args) {
                    out.push(Interp {
                        via: Via::Method(d, t),
                        ..i
                    });
                }
            }
        }
        out
    }

    /// The object of method call `obj.m`, of type `t`, dereferenced when `t`
    /// is an access type (8.3).
    pub(crate) fn method_object(&mut self, scope: ScopeId, obj: &ast::Name, t: TypeId) -> ExprId {
        let p = self.resolve_prefix(scope, obj, t);
        match self.method_target(t) {
            Some((pt, true)) if !self.is_poison_expr(p) => {
                let span = self.exprs[p].span;
                self.add_expr(Expr {
                    kind: ExprKind::Deref { prefix: p },
                    subtype: self.types[pt].first_subtype,
                    span,
                    stat: Staticness::None,
                    value: None,
                })
            }
            _ => p,
        }
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
            // An access value is appropriate for its designated type (8.1, alloc.rs).
            let fits = i.ty == t
                || i.ty == self.poison.ty
                || self.designated(i.ty).is_some_and(|d| self.base(d) == t);
            if fits && !best.iter().any(|b| b.via == i.via) {
                best.push(i);
            }
        }
        best.sort_by_key(|i| i.convs);
        let id = match best.as_slice() {
            [] => return self.poison_expr(),
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
                return self.poison_expr();
            }
        };
        self.deref_to(id, t)
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
                    Via::Method(d, t) => {
                        let ast::NameKind::Selected { prefix: obj, .. } = &prefix.kind else {
                            return self.poison_expr();
                        };
                        let o = self.method_object(scope, obj, t);
                        self.build_call(scope, d, args, span, Some(o))
                    }
                    Via::Index(t) => self.build_index(scope, prefix, t, args, span),
                    Via::Slice(t) => self.build_slice(scope, prefix, t, args, span),
                    _ => self.poison_expr(),
                }
            }
            (ast::NameKind::Selected { prefix, .. }, Via::Method(d, t)) => {
                let o = self.method_object(scope, prefix, t);
                self.build_call(scope, d, &[], span, Some(o))
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
            (ast::NameKind::Selected { prefix, .. }, Via::Deref(t)) => {
                let p = self.resolve_prefix(scope, prefix, t);
                self.deref(p, span)
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
            }) => self.range_name(scope, n, index),
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
        let Some((e, hargs, stat)) = self.call_args(scope, d, args, span) else {
            return self.poison_expr();
        };
        let s = self.decls[e]
            .kind
            .subprogram()
            .expect("call_args checked the callee");
        let ret = s.ret.unwrap_or(self.poison.subtype);
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        let id = self.add_expr(Expr {
            kind: ExprKind::Call {
                callee: e,
                args: hargs,
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

    /// The callee entity and its actuals in formal order, with their combined staticness;
    /// `None` when already reported. Procedure calls use this directly (no call expression).
    pub(crate) fn call_args(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        args: &[ast::AssocElem],
        span: Span,
    ) -> Option<(DeclId, Box<[Arg]>, Staticness)> {
        let e = self.entity(d);
        let s = self.decls[e].kind.subprogram()?;
        let formals: Vec<(Symbol, SubtypeId)> =
            s.params.iter().map(|p| (p.name.key, p.subtype)).collect();
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        let map = assoc_map(&formals.iter().map(|f| f.0).collect::<Vec<_>>(), args)?;
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
                        let id = self.resolve_in(scope, x, *ft);
                        stat = stat.min(self.exprs[id].stat);
                        Actual::Expr(id)
                    }
                    ast::Actual::Open(_) => Actual::Open,
                    other => {
                        let sp = actual_span(other).unwrap_or(span);
                        self.unsupported(sp, "this kind of actual", "S2");
                        return None;
                    }
                },
                None => Actual::Default,
            };
            hargs.push(Arg {
                formal: k as u16,
                actual,
            });
        }
        Some((e, hargs.into(), stat))
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
            return self.build_user_attr(scope, prefix, attr, span);
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
        if a == PredefAttr::Reflect {
            // A mirror is a fresh access value (16.2): never static.
            stat = Staticness::None;
        }
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

    /// Report why attribute name `prefix'attr` has no interpretation (E1621,
    /// E1007 through [`attr::select`]). Returns `false` when it found nothing
    /// to report.
    pub(crate) fn report_attr(&mut self, scope: ScopeId, prefix: &ast::Name, attr: Ident) -> bool {
        let ps = match self.attr_prefixes(scope, prefix) {
            Ok(ps) => ps,
            Err(r) => {
                self.report_name(scope, r);
                return true;
            }
        };
        if PredefAttr::from_key(attr.key).is_none() {
            let declared = matches!(
                self.lookup(scope, attr.key),
                Visible::One(d) if matches!(self.decls[self.entity(d)].kind, DeclKind::Attribute { .. })
            );
            let mut d = if declared {
                Diagnostic::error(
                    diag::UNKNOWN_ATTRIBUTE,
                    format!("this prefix has no value for attribute `{}`", attr.text),
                )
                .with_primary(attr.span, "no specification gives it one")
                .with_note("a user-defined attribute is read from the named entity an attribute specification decorates (LRM 7.2)")
            } else {
                Diagnostic::error(
                    diag::UNKNOWN_ATTRIBUTE,
                    format!("unknown attribute `{}`", attr.text),
                )
                .with_primary(attr.span, "not a predefined or declared attribute")
            };
            if !declared
                && let Some(best) =
                    crate::suggest::closest(attr.key.as_str(), attr::TABLE.iter().map(|i| i.name))
            {
                d = d.with_help(format!("did you mean `{best}`?"));
            }
            self.report(d);
            return true;
        }
        for (p, _) in &ps {
            if attr::select(self, attr, p, attr.span).is_err() {
                return true;
            }
        }
        false
    }

    /// `x'a` for a user-defined attribute `a` (6.7): the value given to the
    /// named entity `x` by a specification (7.2).
    fn build_user_attr(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        span: Span,
    ) -> ExprId {
        let Ok(ps) = self.attr_prefixes(scope, prefix) else {
            return self.poison_expr();
        };
        let found = ps.iter().find_map(|(_, src)| match *src {
            PSrc::Entity(d) | PSrc::Object(d, _) => {
                attr::user_value(self, self.entity(d), attr.key)
            }
            _ => None,
        });
        let Some(v) = found else {
            return self.poison_expr();
        };
        let value = self.attr_values[v].value;
        let subtype =
            attr::spec_subtype(self, self.attr_values[v].attr).unwrap_or(self.exprs[value].subtype);
        let (stat, folded) = (self.exprs[value].stat, self.exprs[value].value);
        self.add_expr(Expr {
            kind: ExprKind::AttrValue { spec: v },
            subtype,
            span,
            stat,
            value: folded,
        })
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

/// Where an attribute prefix interpretation gets its type from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PSrc {
    /// A type mark.
    Type(SubtypeId),
    /// An object name (and the object's type).
    Object(DeclId, TypeId),
    /// A value of this type that is not a named object.
    Value(TypeId),
    /// A named entity that is not an object or type.
    Entity(DeclId),
}

/// The 0-based dimension an integer literal argument names; 0 otherwise.
fn literal_dim(args: &[ast::AssocElem]) -> u32 {
    match args {
        [
            ast::AssocElem {
                actual:
                    ast::Actual::Expr(ast::Expr {
                        kind:
                            ast::ExprKind::Literal(ast::Literal {
                                kind: ast::LiteralKind::Abstract(v),
                                ..
                            }),
                        ..
                    }),
                ..
            },
        ] => v.as_str().parse::<u32>().map_or(0, |d| d.saturating_sub(1)),
        _ => 0,
    }
}

/// Does `n` arguments fit the parameter of `info` for this prefix? Object
/// forms of `'image`/`'pos` (2019) take none.
fn arity_ok(info: &AttrInfo, p: &attr::Prefix, n: usize) -> bool {
    use attr::Param as P;
    match info.param {
        P::None => n == 0,
        P::Dim | P::Time => n <= 1,
        P::PrefixValue if matches!(p.form, PrefixForm::Object { .. }) => n == 0,
        P::PrefixValue | P::String | P::AnyInteger => n == 1,
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
