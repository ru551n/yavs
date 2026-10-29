//! Name interpretation: simple and expanded names, `Apply` (calls, conversions,
//! attribute functions), predefined attributes (sem.md 1.5, 3.3).
//!
//! Both passes go through [`Db::classify`], which resolves the non-overloaded
//! prefix chain of a name without reporting; the top-down pass reports.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use super::{CandSet, Expected, Interp, Via};
use crate::attr::{self, AttrInfo, Kind, PredefAttr, PrefixForm};
use crate::db::{Db, Reported};
use crate::decl::{Alias, DeclKind, char_key, operator_key};
use crate::diag;
use crate::hir::{Actual, Arg, AttrPrefix, Expr, ExprKind, UnitHir};
use crate::ids::{ConstId, DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use crate::loader::UnitKind;
use crate::scope::{ScopeKind, Visible};
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
            let r = match self.lookup(scope, key) {
                Visible::One(d) => NameRes::One(d),
                Visible::Overloads(ds) => NameRes::Overloads(ds.to_vec()),
                Visible::None => NameRes::Missing(ident),
                Visible::Hidden(c) => NameRes::Conflict(ident, c),
            };
            return self.rev_gate(r, n.span, n.span);
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
                    // `p.x` inside package `p` or its body: one region (12.1, #854).
                    NameRes::One(d) if let Some(regions) = self.own_package(scope, d) => {
                        let at = suffix_span(suffix, n.span);
                        let r = self.own_lookup(regions, at, key);
                        self.rev_gate(r, n.span, at)
                    }
                    NameRes::One(d) => {
                        let region = match self.decls[self.entity(d)].kind {
                            DeclKind::Library(l) => Region::Library(l),
                            DeclKind::Unit(_)
                            | DeclKind::PackageInst(_)
                            | DeclKind::GenericPackage(_)
                            | DeclKind::Package { .. }
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
                            DeclKind::Unit(u)
                                if matches!(
                                    self.units[u].kind,
                                    crate::loader::UnitKind::Entity
                                        | crate::loader::UnitKind::Architecture
                                ) =>
                            {
                                match self.enclosing_unit_region(scope, d) {
                                    Some(s) => Region::Scope(s),
                                    // Its declarations are selected by name only within it (8.3).
                                    None => {
                                        return NameRes::NotInRegion(
                                            self.units[u].scope,
                                            suffix_span(suffix, n.span),
                                            key,
                                        );
                                    }
                                }
                            }
                            DeclKind::Object(_) | DeclKind::Alias(Alias::Object { .. }) => {
                                return NameRes::Value;
                            }
                            DeclKind::Error => return NameRes::Error,
                            DeclKind::Type(t)
                                if matches!(self.types[t].kind, TypeKind::Protected { .. }) =>
                            {
                                return self.protected_expanded(
                                    scope,
                                    d,
                                    t,
                                    suffix_span(suffix, n.span),
                                    key,
                                );
                            }
                            DeclKind::Label(_) => match self.construct_region(scope, &[d]) {
                                Some((s, true)) => Region::Scope(s),
                                Some((s, false)) => {
                                    return NameRes::NotInRegion(
                                        s,
                                        suffix_span(suffix, n.span),
                                        key,
                                    );
                                }
                                None => {
                                    return NameRes::Unsupported(
                                        n.span,
                                        "expanded names into this kind of region",
                                        "S5",
                                    );
                                }
                            },
                            // 8.3: a type, subtype or component is not a declarative region.
                            DeclKind::Type(_)
                            | DeclKind::Subtype(_)
                            | DeclKind::GenericType(_)
                            | DeclKind::Component(_)
                            | DeclKind::EnumLit { .. }
                            | DeclKind::PhysUnit { .. }
                            | DeclKind::Attribute { .. } => {
                                let what = self.decls[d].name;
                                self.report(
                                    Diagnostic::error(
                                        diag::NOT_A_PACKAGE,
                                        format!("`{what}` is not a region an expanded name can select from"),
                                    )
                                    .with_primary(prefix.span, "not a library, design unit, subprogram or labelled statement")
                                    .with_note("LRM 8.3: the prefix of an expanded name denotes a construct that is a declarative region"),
                                );
                                return NameRes::Error;
                            }
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
                            Region::Scope(s) => {
                                let at = suffix_span(suffix, n.span);
                                let r = self.in_region(s, at, key);
                                self.rev_gate(r, n.span, at)
                            }
                        }
                    }
                    // `f.v` names `v` in subprogram `f` only inside `f` (8.3); outside it
                    // is a selection from a call's value, unless `v` is local to `f`.
                    NameRes::Overloads(ds) => match self.construct_region(scope, &ds) {
                        Some((s, true)) => self.in_region(s, suffix_span(suffix, n.span), key),
                        Some((s, false))
                            if self.scopes[s].names.contains_key(&key)
                                && !ds.iter().any(|&d| self.returns_composite(d)) =>
                        {
                            NameRes::NotInRegion(s, suffix_span(suffix, n.span), key)
                        }
                        _ => NameRes::Value,
                    },
                    NameRes::Value => NameRes::Value,
                    other => other,
                }
            }
            ast::NameKind::External(_)
            | ast::NameKind::Apply { .. }
            | ast::NameKind::Attribute { .. } => NameRes::Value,
            ast::NameKind::GenericMap { prefix, args } => {
                self.call_instance(scope, prefix, args, n.span)
            }
            _ => NameRes::Unsupported(n.span, "this form of name", "S4"),
        }
    }

    /// The suffix `key` of an expanded name looked up in region `s`.
    fn in_region(&self, s: ScopeId, span: Span, key: Symbol) -> NameRes {
        match self.scopes[s].names.get(&key) {
            Some(ds) if ds.len() == 1 && !self.is_overloadable(ds[0]) => NameRes::One(ds[0]),
            Some(ds) if !ds.is_empty() => NameRes::Overloads(ds.clone()),
            _ => NameRes::NotInRegion(s, span, key),
        }
    }

    /// `pt.x` with protected type `pt` as prefix: `x` declared immediately within the type's
    /// declaration or body (one declarative region, 12.1), legal only inside them (8.3, #250).
    fn protected_expanded(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        t: TypeId,
        at: Span,
        key: Symbol,
    ) -> NameRes {
        let TypeKind::Protected {
            decl_scope, body, ..
        } = self.types[t].kind
        else {
            self.ice(at, "protected type prefix without a protected type");
            return NameRes::Error;
        };
        let region = body
            .filter(|&b| self.is_within(scope, b))
            .unwrap_or(decl_scope);
        if !matches!(self.construct_region(scope, &[d]), Some((_, true))) {
            let home = body
                .filter(|&b| self.scopes[b].names.contains_key(&key))
                .unwrap_or(decl_scope);
            return NameRes::NotInRegion(home, at, key);
        }
        self.own_lookup((region, decl_scope, body), at, key)
    }

    /// `key` seen from `region` (one of the two parts `decl` and `body` of one declarative
    /// region), keeping only the declarations of those parts: one walk for hiding and
    /// overloading (a parameter `n` of a method does not hide `pt.n`).
    fn own_lookup(
        &mut self,
        (region, decl, body): (ScopeId, ScopeId, Option<ScopeId>),
        at: Span,
        key: Symbol,
    ) -> NameRes {
        let own = |db: &Db, x: DeclId| [Some(decl), body].contains(&Some(db.decls[x].scope));
        match self.lookup(region, key) {
            Visible::One(x) if own(self, x) => NameRes::One(x),
            Visible::Overloads(ds) => {
                let ds: Vec<DeclId> = ds.iter().copied().filter(|&x| own(self, x)).collect();
                if ds.is_empty() {
                    NameRes::NotInRegion(region, at, key)
                } else {
                    NameRes::Overloads(ds)
                }
            }
            _ => NameRes::NotInRegion(region, at, key),
        }
    }

    /// When `d` names a package (its declaration or its body, a design unit or nested) and
    /// `scope` lies within it: the part `scope` is in, the declaration's region and the body's.
    fn own_package(
        &self,
        scope: ScopeId,
        d: DeclId,
    ) -> Option<(ScopeId, ScopeId, Option<ScopeId>)> {
        let (decl, body) = match self.decls[self.entity(d)].kind {
            // `hir` is set only once the unit is analysed.
            DeclKind::Unit(u) => match self.units[u].kind {
                UnitKind::PackageBody => {
                    let package = *self.units[u].deps.first()?;
                    (self.units[package].scope, Some(self.units[u].scope))
                }
                UnitKind::Package if !matches!(self.units[u].hir, UnitHir::PackageInst(_)) => {
                    (self.units[u].scope, None)
                }
                _ => return None,
            },
            DeclKind::Package { scope, body, .. } => (scope, body),
            _ => return None,
        };
        if !self.is_within(scope, decl) {
            return None;
        }
        // A unit body's region lies under its context region, under the package's.
        let body = body.or_else(|| {
            let mut cur = Some(scope);
            while let Some(s) = cur {
                let up = self.scopes[s].parent;
                if self.scopes[s].kind == ScopeKind::PackageBody
                    && up.and_then(|c| self.scopes[c].parent) == Some(decl)
                {
                    return Some(s);
                }
                cur = up;
            }
            None
        });
        let region = body.filter(|&b| self.is_within(scope, b)).unwrap_or(decl);
        Some((region, decl, body))
    }

    /// The region of the construct (labelled statement or subprogram) one of `ds` names,
    /// and whether it encloses `scope` (8.3: only then is an expanded name into it legal).
    pub(crate) fn construct_region(
        &self,
        scope: ScopeId,
        ds: &[DeclId],
    ) -> Option<(ScopeId, bool)> {
        let owned = |s: ScopeId| self.scopes[s].owner.is_some_and(|o| ds.contains(&o));
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if owned(s) {
                return Some((s, true));
            }
            cur = self.scopes[s].parent;
        }
        ds.iter()
            .find_map(|&d| match self.decls[d].kind.subprogram()?.body {
                crate::decl::Body::Hir { scope, .. } => Some(scope),
                _ => None,
            })
            // ponytail: linear scan for a label's region, only reached for a label prefix
            // outside its statement (an error); index labels if that changes.
            .or_else(|| self.scopes.iter().map(|(s, _)| s).find(|&s| owned(s)))
            .map(|s| (s, false))
    }

    /// A function whose result could be selected from (`f.x` of a record or an access).
    fn returns_composite(&self, d: DeclId) -> bool {
        self.decls[d]
            .kind
            .subprogram()
            .and_then(|s| s.ret)
            .is_some_and(|r| {
                matches!(
                    self.types[self.base(r)].kind,
                    TypeKind::Record { .. } | TypeKind::Access { .. }
                )
            })
    }

    /// Report why a name has no usable interpretation. `span` is the name's, `what` says what
    /// the caller needed ("a function name"). `One` and `Overloads` are usable names: a caller
    /// reaching this with one has a bug (ICE). `Value` is an object part where `what` was needed.
    pub(crate) fn report_name(
        &mut self,
        scope: ScopeId,
        r: NameRes,
        span: Span,
        what: &str,
    ) -> Reported {
        match r {
            NameRes::Missing(i) => self.undeclared(scope, i),
            NameRes::Conflict(i, c) => self.conflict(i, &c),
            NameRes::Unsupported(span, what, slice) => self.unsupported(span, what, slice),
            NameRes::NotInRegion(s, span, key) => self.not_in_region(s, span, key),
            NameRes::Error => Reported,
            NameRes::Uninstantiated(d, span) => self.uninstantiated(d, span),
            NameRes::Value => self.error(
                Diagnostic::error(diag::TYPE_MISMATCH, format!("expected {what}"))
                    .with_primary(span, format!("not {what}")),
            ),
            NameRes::One(_) | NameRes::Overloads(_) => {
                self.ice(span, "a usable name reached report_name")
            }
        }
    }

    /// The subtype a type mark name denotes, without reporting.
    pub(crate) fn quiet_type_mark(&mut self, scope: ScopeId, n: &ast::Name) -> Option<SubtypeId> {
        // `x'element`, `x'subtype`, `t'base` denote subtypes (16.2), so `rows'element'range`
        // is an attribute of a type mark.
        if let ast::NameKind::Attribute { prefix, attr } = &n.kind
            && matches!(attr.key.as_str(), "subtype" | "element" | "base")
        {
            return self.attribute_type_mark(scope, prefix, *attr).ok();
        }
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
            ast::NameKind::External(x) => match self.external_expr(scope, x, n) {
                Some(e) => CandSet::Types(smallvec::smallvec![Interp::new(
                    self.base(self.exprs[e].subtype),
                    Via::Object,
                )]),
                None => self.poison_cands(),
            },
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
                Some(t) => CandSet::Types(smallvec::smallvec![Interp::new(t, Via::Object)]),
                None => match self.decls[self.entity(d)].kind {
                    DeclKind::PhysUnit { ty, .. } => {
                        CandSet::Types(smallvec::smallvec![Interp::new(ty, Via::Literal)])
                    }
                    DeclKind::Error => self.poison_cands(),
                    _ => CandSet::Types(Default::default()),
                },
            },
            NameRes::Overloads(ds) => {
                CandSet::Types(ds.iter().filter_map(|&d| self.value_interp(d, 0)).collect())
            }
            NameRes::Error => self.poison_cands(),
            _ => CandSet::Types(Default::default()),
        }
    }

    /// The HIR of external name `x` (8.7), built once per AST node; `None` when reported.
    fn external_expr(
        &mut self,
        scope: ScopeId,
        x: &ast::ExternalName,
        n: &ast::Name,
    ) -> Option<ExprId> {
        let key = n as *const ast::Name as usize;
        if let Some(&e) = self.ext_memo.get(&key) {
            return (!self.is_poison_expr(e)).then_some(e);
        }
        let built = (|| {
            let st = self.subtype_indication(scope, &x.subtype)?;
            let mut indexes = Vec::with_capacity(x.path.elements.len());
            for el in &x.path.elements {
                indexes.push(
                    el.index
                        .as_ref()
                        .map(|i| self.static_expr(scope, i, Expected::Any)),
                );
            }
            let mut cx = crate::external::ExternalCx::default();
            let mut cur = Some(scope);
            while let Some(s) = cur {
                use crate::scope::ScopeKind as K;
                match self.scopes[s].kind {
                    K::Entity | K::Architecture | K::Block | K::Generate => {
                        cx.in_concurrent_region = true
                    }
                    K::Subprogram if !cx.in_concurrent_region => {
                        if let Some(o) = self.scopes[s].owner
                            && let Some(sp) = self.decls[o].kind.subprogram()
                            && matches!(
                                sp.kind,
                                crate::decl::SubprogramKind::Function { pure: true }
                            )
                        {
                            cx.in_pure_function = true;
                        }
                    }
                    _ => {}
                }
                cur = self.scopes[s].parent;
            }
            self.external_name(x, n.span, st, &indexes, cx)
        })();
        let e = built.unwrap_or(self.poison.expr);
        self.ext_memo.insert(key, e);
        (!self.is_poison_expr(e)).then_some(e)
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
            CandSet::Class(_) | CandSet::Alloc(_) => Ok(Vec::new()),
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
        let mut out = super::Interps::new();
        for &t in &ts {
            // method_decls follows an access to a protected type itself.
            for d in self.method_decls(t, key) {
                if let Some(i) = self.value_interp(d, 0) {
                    out.push(Interp {
                        via: Via::Method(d, t),
                        ..i
                    });
                }
            }
        }
        for t in self.with_designated(ts) {
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

    /// `prefix.id` where the prefix is a value of one record type that has no element `id`:
    /// report it (with a suggestion) and return true.
    pub(crate) fn unknown_element(&mut self, scope: ScopeId, n: &ast::Name) -> bool {
        let ast::NameKind::Selected { prefix, suffix } = &n.kind else {
            return false;
        };
        let ast::Suffix::Ident(id) = suffix else {
            return false;
        };
        if !matches!(self.classify(scope, n), NameRes::Value) {
            return false;
        }
        let Ok(ts) = self.prefix_types(scope, prefix) else {
            return false;
        };
        let recs = self.with_designated(ts);
        let [t] = recs[..] else { return false };
        let TypeKind::Record { elems } = &self.types[t].kind else {
            return false;
        };
        let mut d = Diagnostic::error(
            diag::NOT_DECLARED,
            format!(
                "{} has no element `{}`",
                super::aggregate::type_name(self, t),
                id.text
            ),
        )
        .with_primary(id.span, "unknown element");
        if let Some(h) = crate::suggest::suggest(
            id.key.as_str(),
            "element",
            elems
                .iter()
                .map(|e| (e.name.key.as_str(), e.name.text.as_str())),
        ) {
            d = d.with_help(h);
        }
        self.report(d);
        true
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
            // A poison declaration was reported where it failed: stay silent (8.2).
            NameRes::One(d) if self.is_poison(self.entity(d)) => Err(NameRes::Error),
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
                self.externals[*x].class == yavs_syntax::ast::ExternalClass::Signal
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
                // `s'delayed(t)`, `'stable`, `'quiet`, `'transaction` denote signals (16.2.4).
                ast::NameKind::Attribute { prefix, attr } => {
                    PredefAttr::from_key(attr.key).is_some_and(|a| a.info().kind == Kind::Signal)
                        && self.ast_root_signal(scope, prefix)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Attributes (16.2 predefined through the `attr::TABLE`, 6.7 user-defined).
    /// `p'range'left` (or `p'range(n)'left`): a range attribute is not a name, but sim1 and sim2
    /// read `'left`, `'right`, `'low`, `'high` and `'ascending` of it as the same attribute of
    /// `p`. Returns `p`, that attribute and the dimension argument (#818). Not of
    /// `'reverse_range`, where the two references give different values.
    fn range_attr_alias(
        prefix: &ast::Name,
        attr: Ident,
    ) -> Option<(&ast::Name, Ident, &[ast::AssocElem])> {
        let (p, r, args) = match &prefix.kind {
            ast::NameKind::Attribute { prefix: p, attr: r } => (p.as_ref(), *r, &[][..]),
            ast::NameKind::Apply { prefix: q, args } if args.len() == 1 => match &q.kind {
                ast::NameKind::Attribute { prefix: p, attr: r } => (p.as_ref(), *r, &args[..]),
                _ => return None,
            },
            _ => return None,
        };
        (r.key.as_str() == "range"
            && matches!(
                attr.key.as_str(),
                "left" | "right" | "low" | "high" | "ascending"
            ))
        .then_some((p, attr, args))
    }

    fn attr_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        args: &[ast::AssocElem],
    ) -> CandSet {
        if args.is_empty()
            && let Some((p, a, dim)) = Self::range_attr_alias(prefix, attr)
        {
            return self.attr_cands(scope, p, a, dim);
        }
        if attr.key.as_str() == "value"
            && args.is_empty()
            && let Some(r) = self.range_attr(scope, prefix)
        {
            // 2019 `R'value` (16.2.6): the range record of the range's type.
            if self.file_rev() < yavs_syntax::Std::Vhdl2019 {
                self.report(
                    Diagnostic::error(
                        diag::NOT_IN_REVISION,
                        "`'value` of a range attribute is not available in VHDL-2008",
                    )
                    .with_primary(attr.span, "needs VHDL-2019"),
                );
                return self.poison_cands();
            }
            return match r.and_then(|(_, t)| self.range_record_of(t, attr.span)) {
                Ok(st) => {
                    CandSet::Types(smallvec::smallvec![Interp::new(self.base(st), Via::Other)])
                }
                Err(_) => self.poison_cands(),
            };
        }
        // No interpretation: the top-down pass reports why (`report_attr`).
        let ps = match self.attr_prefixes(scope, prefix) {
            Ok(ps) => ps,
            Err(NameRes::Error) => return self.poison_cands(),
            Err(_) => return CandSet::Types(Default::default()),
        };
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return match self.user_attr_type(scope, prefix, &ps, attr.key) {
                Some(t) if args.is_empty() => {
                    CandSet::Types(smallvec::smallvec![Interp::new(t, Via::Attr)])
                }
                // The value of a user attribute is indexed or sliced like any
                // other array value (6.7, 8.4, #657).
                Some(t) => {
                    let slice = self.is_slice_arg(scope, args);
                    match self.index_interp(scope, t, args, slice) {
                        Some(i) => CandSet::Types(smallvec::smallvec![i]),
                        None => CandSet::Types(Default::default()),
                    }
                }
                None => CandSet::Types(Default::default()),
            };
        };
        let info = a.info();
        let dim = literal_dim(args);
        // `p'reflect` of an access object mirrors the access value (class_access), not the
        // designated object its implicit dereference would give (16.2, LRM 2019 16.11).
        let derefs: Vec<TypeId> = if a == PredefAttr::Reflect {
            ps.iter()
                .filter_map(|(_, s)| match *s {
                    PSrc::Object(_, t) | PSrc::Value(t) => self.designated(t).map(|d| self.base(d)),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut out = super::Interps::new();
        for (p, src) in &ps {
            if matches!(*src, PSrc::Value(t) if derefs.contains(&t)) {
                continue;
            }
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

    /// The declaration a type-mark prefix names: its own, so an attribute of a
    /// subtype is found on the subtype, not on its base type (7.2).
    fn type_mark_decl(&mut self, scope: ScopeId, prefix: &ast::Name) -> Option<DeclId> {
        match self.classify(scope, prefix) {
            NameRes::One(d) => match self.decls[self.entity(d)].kind {
                DeclKind::Type(_) | DeclKind::GenericType(_) | DeclKind::Subtype(_) => Some(d),
                _ => None,
            },
            _ => None,
        }
    }

    /// The type of user-defined attribute `key` of the named entity a prefix
    /// denotes, if it has a value for it (7.2). Any overload with a value counts.
    fn user_attr_type(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        ps: &[(attr::Prefix, PSrc)],
        key: Symbol,
    ) -> Option<TypeId> {
        ps.iter().find_map(|(_, src)| {
            let v = match *src {
                PSrc::Entity(d) | PSrc::Object(d, _) => self.user_attr_value(d, key)?,
                // A type mark: the attribute of the type or subtype declaration.
                PSrc::Type(_) => {
                    let d = self.type_mark_decl(scope, prefix)?;
                    self.user_attr_value(d, key)?
                }
                PSrc::Value(_) => return None,
            };
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
        if let ast::NameKind::Attribute { prefix: p, attr } = &prefix.kind
            && takes_args(attr)
        {
            return self.attr_cands(scope, p, *attr, args);
        }
        if let Some(st) = self.quiet_type_mark(scope, prefix) {
            // Type conversion (9.3.6), also to `x'subtype`.
            return CandSet::Types(smallvec::smallvec![Interp::new(self.base(st), Via::Other)]);
        }
        let slice = self.is_slice_arg(scope, args);
        let mut out = super::Interps::new();
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
        let formals: smallvec::SmallVec<[(Symbol, TypeId, bool); 8]> = s
            .params
            .iter()
            .map(|p| (p.name.key, self.base(p.subtype), p.default.is_some()))
            .collect();
        let map = assoc_map(
            &formals
                .iter()
                .map(|f| f.0)
                .collect::<smallvec::SmallVec<[yavs_syntax::Symbol; 8]>>(),
            args,
        )
        .ok()?;
        let mut convs = 0;
        let mut anon_bound: Vec<(TypeId, TypeId)> = Vec::new();
        for (i, (_, ft, has_default)) in formals.iter().enumerate() {
            let whole = match &map[i] {
                Slot::Whole(a) => Some(&args[*a].actual),
                Slot::None => None,
                Slot::Parts(parts) => {
                    convs += self.parts_cost(scope, d, i, args, parts)?;
                    continue;
                }
                Slot::Conv(a) => {
                    convs += self.conv_cost(scope, d, i, args, *a)?;
                    continue;
                }
            };
            match whole {
                Some(ast::Actual::Expr(x)) => {
                    let c = self.cands(scope, x);
                    // An anonymous parameter type takes any type of its class (#283).
                    convs += if self.anonymous_param(e, *ft) {
                        if !anon_bound.iter().any(|b| b.0 == *ft)
                            && let Some(t) = self.anonymous_binding(&c, *ft)
                        {
                            anon_bound.push((*ft, t));
                        }
                        self.anonymous_cost(&c, *ft)?
                    } else {
                        self.best_cost(&c, Expected::Type(*ft))?
                    };
                }
                // 6.5.6.2: `open` stands for the default expression.
                None | Some(ast::Actual::Open(_)) if *has_default => {}
                _ => return None,
            }
        }
        // A result of an anonymous parameter type is the type its actual binds.
        let ret = if self.anonymous_param(e, ret) {
            anon_bound.iter().find(|b| b.0 == ret)?.1
        } else {
            ret
        };
        Some(Interp {
            convs,
            ..Interp::new(ret, Via::Call(d))
        })
    }

    /// The conversion cost of the sub-element associations `parts` of formal `k` of
    /// subprogram `d` (6.5.7.1): each actual against the type of its formal part.
    /// `None` if one does not fit.
    pub(crate) fn parts_cost(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        k: usize,
        args: &[ast::AssocElem],
        parts: &[usize],
    ) -> Option<u32> {
        let s = self.decls[self.entity(d)].kind.subprogram()?;
        let p = s.params.get(k)?.decl?;
        // The formal part names this candidate's formal: resolve it in a region that
        // holds only the formal (L42), with fresh memos, since another candidate's
        // formal of the same name has another type.
        let fscope = self.add_scope(crate::scope::ScopeKind::Block, Some(scope), None);
        let key = self.decls[p].name.key();
        self.scopes[fscope].names.insert(key, vec![p]);
        let saved = (
            std::mem::take(&mut self.memo),
            std::mem::take(&mut self.name_memo),
        );
        let mut types = Vec::with_capacity(parts.len());
        for &a in parts {
            let t = match args[a]
                .formal
                .as_ref()
                .map(|f| self.name_cands_uncached(fscope, f))
            {
                Some(CandSet::Types(is)) if is.len() == 1 => Some(is[0].ty),
                _ => None,
            };
            types.push(t);
        }
        (self.memo, self.name_memo) = saved;
        let mut convs = 0;
        for (&a, t) in parts.iter().zip(types) {
            let ast::Actual::Expr(x) = &args[a].actual else {
                return None;
            };
            let c = self.cands(scope, x);
            convs += self.best_cost(&c, Expected::Type(t?))?;
        }
        Some(convs)
    }

    /// Run `g` in a region inside `scope` that holds only formal `p` (L42), with fresh
    /// memos: a formal part names one candidate's formal, and another candidate's formal of
    /// the same name has another type.
    fn in_formal_region<T>(
        &mut self,
        scope: ScopeId,
        p: DeclId,
        g: impl FnOnce(&mut Db, ScopeId) -> T,
    ) -> T {
        let fscope = self.add_scope(crate::scope::ScopeKind::Block, Some(scope), None);
        let key = self.decls[p].name.key();
        self.scopes[fscope].names.insert(key, vec![p]);
        let saved = (
            std::mem::take(&mut self.memo),
            std::mem::take(&mut self.name_memo),
        );
        let r = g(self, fscope);
        (self.memo, self.name_memo) = saved;
        r
    }

    /// The types the formal part `conv(f)` of association `a` can have, for formal `k` of
    /// subprogram `d` (6.5.7.1).
    fn conv_types(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        k: usize,
        a: &ast::AssocElem,
    ) -> Vec<TypeId> {
        let p = self.decls[self.entity(d)]
            .kind
            .subprogram()
            .and_then(|s| s.params.get(k)?.decl);
        let (Some(p), Some(f)) = (p, a.formal.as_ref()) else {
            return Vec::new();
        };
        self.in_formal_region(scope, p, |db, fs| match db.name_cands_uncached(fs, f) {
            CandSet::Types(is) => is.iter().map(|i| i.ty).collect(),
            _ => Vec::new(),
        })
    }

    /// The conversion cost of `conv(f) => actual` (association `a`) for formal `k` of
    /// subprogram `d`. The actual part has the formal's type when it converts on the way
    /// in, else the formal conversion's type; either fits here and `conv_actual` checks
    /// that they agree with the mode (E1313), so the error is precise. `None` if neither
    /// fits.
    pub(crate) fn conv_cost(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        k: usize,
        args: &[ast::AssocElem],
        a: usize,
    ) -> Option<u32> {
        let ft = self.base(
            self.decls[self.entity(d)]
                .kind
                .subprogram()?
                .params
                .get(k)?
                .subtype,
        );
        let ast::Actual::Expr(x) = &args[a].actual else {
            return Some(0);
        };
        let types = self.conv_types(scope, d, k, &args[a]);
        let c = self.cands(scope, x);
        std::iter::once(ft)
            .chain(types)
            .filter_map(|t| self.best_cost(&c, Expected::Type(t)))
            .min()
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
            (ast::NameKind::External(x), _) => {
                self.external_expr(scope, x, n).unwrap_or(self.poison.expr)
            }
            (ast::NameKind::Attribute { prefix, attr }, _) => {
                self.build_attr(scope, prefix, *attr, &[], i, span)
            }
            (ast::NameKind::Apply { prefix, args }, _) => {
                if let ast::NameKind::Attribute { prefix: p, attr } = &prefix.kind
                    && takes_args(attr)
                {
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
                let subtype = self.field_subtype(self.exprs[p].subtype, elem);
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
                        let unit = self.decls[self.entity(d)].name.key();
                        self.note_time_unit(ty, unit, scale, false, span);
                        self.lit_expr(ConstValue::Phys(scale), st, span)
                    }
                    _ => self.poison_expr(),
                },
                _ => self.poison_expr(),
            },
            (_, Via::Object) => match self.classify(scope, n) {
                NameRes::One(d) => {
                    self.check_deferred_read(scope, d, span);
                    self.object_expr(d, span)
                }
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
        for (dim, &id) in ids.iter().enumerate() {
            if let Some(v) = self.value(id).and_then(|v| v.as_int())
                && let Some(r) = crate::expr::aggregate::subtype_range(self, pst, dim)
                && !r.contains(v)
            {
                let ix = self.base(indexes[dim]);
                let msg = format!(
                    "index {} is outside range {}",
                    self.index_image(ix, v),
                    self.range_image(ix, r)
                );
                self.warn_static_index(span, msg);
            }
        }
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
        // A null slice may have any bounds (8.5).
        if let Some(s) = crate::expr::aggregate::range_value(self, &range)
            && !s.is_null()
            && let Some(r) = crate::expr::aggregate::subtype_range(self, pst, 0)
            && !(r.contains(s.left) && r.contains(s.right))
        {
            let msg = format!(
                "slice {} is outside range {}",
                self.range_image(index, s),
                self.range_image(index, r)
            );
            self.warn_static_index(span, msg);
        }
        let elem_c = match &self.subtypes[pst].constraint {
            crate::types::Constraint::Array { elem, .. } => elem.clone(),
            _ => Box::new(crate::types::Constraint::None),
        };
        let mut stat = self.exprs[p]
            .stat
            .min(crate::expr::aggregate::range_stat(self, &range));
        // Only slices of scalar elements fold (stat.rs): a slice of composites stays globally static.
        if let TypeKind::Array { elem, .. } = &self.types[t].kind
            && !self.types[self.base(*elem)].kind.is_scalar()
        {
            stat = stat.min(Staticness::Globally);
        }
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
        let id = self.add_expr(Expr {
            kind: ExprKind::Slice { prefix: p, range },
            subtype,
            span,
            stat,
            value: None,
        });
        if stat == Staticness::Locally {
            self.fold_into(id);
        }
        id
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
                        // 9.4.3: every constant is a globally static primary, whatever
                        // its initial value; only a locally static one is also locally static.
                        // ponytail: not inside a process or subprogram, where elaboration
                        // cannot know the value (a driver's index, say): keeps the initial
                        // value's staticness there (#492).
                        let mut sequential = false;
                        let mut cur = Some(self.decls[d].scope);
                        while let Some(sc) = cur {
                            sequential |= matches!(
                                self.scopes[sc].kind,
                                crate::scope::ScopeKind::Process
                                    | crate::scope::ScopeKind::Subprogram
                            );
                            cur = self.scopes[sc].parent;
                        }
                        (
                            if sequential {
                                e.stat
                            } else {
                                e.stat.max(Staticness::Globally)
                            },
                            if e.stat == Staticness::Locally {
                                e.value
                            } else {
                                None
                            },
                        )
                    }
                    // Loop parameters are not static; generate parameters are globally static (8.1, sim1 #856).
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
        let value = value.and_then(|c| self.rebound(c, subtype));
        let id = self.add_expr(Expr {
            kind: ExprKind::Obj(d),
            subtype,
            span,
            stat,
            value,
        });
        // A reference to an `out` port or parameter is a read unless a write context says
        // otherwise (LRM 1993 4.3.2; `Db::not_read`).
        if matches!(&self.decls[d].kind, DeclKind::Object(o) if o.mode == Some(crate::hir::Mode::Out))
            && self
                .cur()
                .is_some_and(|c| c.file_rev == crate::LangRev::Vhdl1993)
        {
            self.out_reads.push(id);
        }
        id
    }

    /// Folded value `c` seen through constrained subtype `st`: the bounds of `st`, not those of
    /// the initial value (a constant takes its subtype's bounds, LRM 6.4.2.2), also for the
    /// array fields of a record. `None` (not folded) when the bounds of `st` are not static
    /// or a length differs, so lowering evaluates the constant and its run-time check reports
    /// a mismatch.
    fn rebound(&mut self, c: ConstId, st: SubtypeId) -> Option<ConstId> {
        let v = self.consts[c].clone();
        let r = self.rebound_value(&v, st)?;
        Some(if r == v { c } else { self.add_const(r) })
    }

    fn rebound_value(&mut self, v: &ConstValue, st: SubtypeId) -> Option<ConstValue> {
        match v {
            ConstValue::Array(a) => {
                let mut want = Vec::with_capacity(a.bounds.len());
                for dim in 0..a.bounds.len() {
                    let Some(r) = crate::expr::aggregate::subtype_range(self, st, dim) else {
                        let constrained = matches!(
                            self.subtypes[st].constraint,
                            crate::types::Constraint::Array { dims: Some(_), .. }
                        );
                        return (!constrained).then(|| v.clone());
                    };
                    want.push((r.left, r.dir, r.right));
                }
                if *a.bounds == *want {
                    return Some(v.clone());
                }
                Some(ConstValue::Array(a.with_bounds(&want)?))
            }
            ConstValue::Record(vs) => {
                let mut out = Vec::with_capacity(vs.len());
                for (i, f) in vs.iter().enumerate() {
                    let fst = self.field_subtype(st, i as u32);
                    out.push(self.rebound_value(f, fst)?);
                }
                Some(ConstValue::Record(out.into()))
            }
            _ => Some(v.clone()),
        }
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
        let method_of = self.forwarded_object(d, method_of);
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

    /// The object a call of `d` on `obj` (`None`: the implicit object) runs on, through
    /// each method alias `alias m is v.m` (5.6.2, 6.6.3): for a private variable `v` of the
    /// protected type declaring the alias, that variable of the object; otherwise `v`.
    pub(crate) fn forwarded_object(
        &mut self,
        mut d: DeclId,
        mut obj: Option<ExprId>,
    ) -> Option<ExprId> {
        loop {
            if let Some(&v) = self.forwarding.get(&d) {
                let span = self.decls[v].span;
                let private =
                    self.is_private_variable(v) && self.decls[v].scope == self.decls[d].scope;
                obj = Some(match obj {
                    Some(object) if private => {
                        let DeclKind::Object(o) = &self.decls[v].kind else {
                            unreachable!("is_private_variable checked an object");
                        };
                        let subtype = o.subtype;
                        self.add_expr(Expr {
                            kind: ExprKind::PrivateVar { object, var: v },
                            subtype,
                            span,
                            stat: Staticness::None,
                            value: None,
                        })
                    }
                    _ => self.object_ref(v, span),
                });
            }
            match &self.decls[d].kind {
                DeclKind::Alias(Alias::NonObject { target }) => d = *target,
                _ => return obj,
            }
        }
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
        // A subprogram with anonymous parameter types: the call goes to its instance for
        // the actuals' types (#283).
        let (e, pre) = self.implicit_instance(scope, e, args, span)?;
        let s = self.decls[e].kind.subprogram()?;
        let formals: smallvec::SmallVec<[(Symbol, SubtypeId); 8]> =
            s.params.iter().map(|p| (p.name.key, p.subtype)).collect();
        let decls: smallvec::SmallVec<[Option<DeclId>; 8]> =
            s.params.iter().map(|p| p.decl).collect();
        let classes: smallvec::SmallVec<[(crate::decl::ObjClass, crate::hir::Mode, Ident); 8]> =
            s.params.iter().map(|p| (p.class, p.mode, p.name)).collect();
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        let map = assoc_map(
            &formals
                .iter()
                .map(|f| f.0)
                .collect::<smallvec::SmallVec<[yavs_syntax::Symbol; 8]>>(),
            args,
        )
        .ok()?;
        let mut hargs = Vec::with_capacity(formals.len());
        // 9.4.3: a pure function call with globally static actuals is globally static
        // (sem folds only the implicit operations, so user calls never become locally static).
        let pure = matches!(s.kind, crate::decl::SubprogramKind::Function { pure: true });
        let mut stat = if implicit || self.in_static_ieee_package(e) {
            Staticness::Locally
        } else if pure {
            Staticness::Globally
        } else {
            Staticness::None
        };
        for (k, (_, ft)) in formals.iter().enumerate() {
            let actual = match &map[k] {
                Slot::Parts(parts) => {
                    let p = decls[k]?;
                    let parts: Vec<(usize, Span)> = parts
                        .iter()
                        .map(|&a| (a, args[a].formal.as_ref().map_or(span, |f| f.span)))
                        .collect();
                    let actual = self.partial(scope, p, args, &parts).ok()?;
                    stat = Staticness::None;
                    actual
                }
                Slot::Conv(a) => {
                    stat = Staticness::None;
                    self.conv_actual(scope, e, k, &args[*a])?
                }
                Slot::Whole(a) => match &args[*a].actual {
                    ast::Actual::Expr(x) => {
                        let id = match pre.get(&k) {
                            Some(&id) => id,
                            None => self.resolve_in(scope, x, *ft),
                        };
                        stat = stat.min(self.exprs[id].stat);
                        let (class, mode, name) = &classes[k];
                        if matches!(x.kind, ast::ExprKind::Name(_))
                            && self.actual_conversion(id).is_some()
                            && !self.actual_conv_allowed(*class, mode, *name, x.span)
                        {
                            return None;
                        }
                        if !self.check_actual_class(*class, mode, *name, id) {
                            return None;
                        }
                        Actual::Expr(id)
                    }
                    ast::Actual::Open(_) => Actual::Open,
                    other => {
                        let sp = actual_span(other).unwrap_or(span);
                        self.unsupported(sp, "this kind of actual", "S2");
                        return None;
                    }
                },
                Slot::None => Actual::Default,
            };
            if classes[k].1 == crate::hir::Mode::Out {
                self.not_read_actual(&actual);
            }
            hargs.push(Arg {
                formal: k as u16,
                actual,
                inertial: false,
            });
        }
        Some((e, hargs.into(), stat))
    }

    /// The subtype a formal-part conversion `fe` gives the formal: its type mark, or the
    /// parameter subtype of the conversion function (5.3.2.2).
    pub fn formal_conv_subtype(&self, fe: ExprId) -> Option<SubtypeId> {
        match &self.exprs[fe].kind {
            ExprKind::Conv { to, .. } => Some(*to),
            ExprKind::Call { callee, .. } => Some(
                self.decls[*callee]
                    .kind
                    .subprogram()?
                    .params
                    .first()?
                    .subtype,
            ),
            _ => None,
        }
    }

    /// `id` as an actual conversion `conv(name)` (6.5.7.1): the conversion's declaration
    /// (a function, or a type for a type conversion) and the object name it converts.
    pub(crate) fn actual_conversion(&self, id: ExprId) -> Option<(DeclId, ExprId)> {
        let (c, x) = match &self.exprs[id].kind {
            ExprKind::Conv { to, operand } => (self.types[self.base(*to)].decl, *operand),
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                ..
            } if args.len() == 1 => match args[0].actual {
                Actual::Expr(x) => (*callee, x),
                _ => return None,
            },
            _ => return None,
        };
        self.name_root(x).is_some().then_some((c, x))
    }

    /// An actual conversion (written `conv(name)`) for parameter `formal` without a formal
    /// conversion (6.5.7.1, 4.2.2.3, E1313): legal for `in` variables only, since data
    /// flowing back to the actual needs a formal conversion. `false` when reported.
    fn actual_conv_allowed(
        &mut self,
        class: crate::decl::ObjClass,
        mode: &crate::hir::Mode,
        formal: Ident,
        span: Span,
    ) -> bool {
        use crate::decl::ObjClass as C;
        use crate::hir::Mode as M;
        let (why, help) = match (class, mode) {
            (C::Signal, _) => (
                "a signal parameter takes no conversion (4.2.2.3)",
                "pass the signal itself and convert inside the subprogram",
            ),
            (C::Variable, M::Out) => (
                "an `out` parameter is not read, so its actual takes no conversion",
                "convert the formal instead: `conv(formal) => actual`",
            ),
            (C::Variable, M::Inout) => (
                "the value of an `inout` parameter also flows back, which needs a formal conversion",
                "add the conversion for the way back: `conv(formal) => conv(actual)`",
            ),
            _ => return true,
        };
        self.report(
            Diagnostic::error(
                diag::CONV_MISUSE,
                format!(
                    "the actual of parameter `{}` cannot be a conversion: {why}",
                    formal.text
                ),
            )
            .with_primary(span, "conversion of the actual")
            .with_help(help),
        );
        false
    }

    /// `conv(f) => actual` (association `a`) for formal `k` of subprogram `e` (6.5.7.1): an
    /// `out` or `inout` variable parameter. The formal part converts the formal to the
    /// actual's type on the way out; for `inout` the actual part converts the actual to the
    /// formal's type on the way in. `None` when reported.
    fn conv_actual(
        &mut self,
        scope: ScopeId,
        e: DeclId,
        k: usize,
        a: &ast::AssocElem,
    ) -> Option<Actual> {
        use crate::decl::ObjClass as C;
        use crate::hir::Mode as M;
        let s = self.decls[e].kind.subprogram()?;
        let prm = s.params.get(k)?;
        let (class, mode, name, st) = (prm.class, prm.mode, prm.name, prm.subtype);
        let Some(p) = prm.decl else {
            self.ice(a.span, "conversion of a formal without a declaration");
            return None;
        };
        let fname = a.formal.as_ref()?;
        let fail = |db: &mut Db, span: Span, msg: String, help: &str| {
            db.report(
                Diagnostic::error(diag::CONV_MISUSE, msg)
                    .with_primary(span, "conversion of the formal")
                    .with_help(help.to_string()),
            );
            None
        };
        let why = match (class, &mode) {
            (C::Variable, M::Out | M::Inout) => None,
            (C::Signal, _) => Some("a signal parameter takes no conversion (4.2.2.3)"),
            (C::Variable | C::Constant, M::In) => {
                Some("data never flows back from an `in` parameter")
            }
            _ => Some("only `out` and `inout` variable parameters take one"),
        };
        if let Some(why) = why {
            return fail(
                self,
                fname.span,
                format!("formal `{}` cannot be converted: {why}", name.text),
                "associate the formal directly",
            );
        }
        let x = match &a.actual {
            ast::Actual::Expr(x) => x,
            ast::Actual::Open(sp) => {
                return fail(
                    self,
                    *sp,
                    format!("a conversion of formal `{}` needs an actual", name.text),
                    "associate a variable, not `open`",
                );
            }
            other => {
                let sp = actual_span(other).unwrap_or(a.span);
                self.unsupported(sp, "this kind of actual", "S2");
                return None;
            }
        };
        let named = matches!(x.kind, ast::ExprKind::Name(_));
        let (obj, copy_in, actual_conv) = if mode == M::Inout {
            // The way in: the actual part converts the actual to the formal's type.
            let call = matches!(&x.kind, ast::ExprKind::Name(n)
                if matches!(n.kind, ast::NameKind::Apply { .. }));
            let fits = call && {
                let c = self.cands(scope, x);
                self.best_cost(&c, Expected::Type(self.base(st))).is_some()
            };
            let id = fits.then(|| self.resolve_in(scope, x, st));
            if id.is_some_and(|id| self.is_poison_expr(id)) {
                return None;
            }
            match id.and_then(|id| Some((id, self.actual_conversion(id)?))) {
                Some((id, (c, o))) => (o, Some(id), Some(c)),
                None => {
                    self.report(
                        Diagnostic::error(
                            diag::CONV_MISUSE,
                            format!(
                                "`inout` parameter `{}` with a formal conversion needs an actual conversion too",
                                name.text
                            ),
                        )
                        .with_primary(x.span, "not a conversion of a variable")
                        .with_help("add the conversion for the way in: `conv(formal) => conv(actual)`"),
                    );
                    return None;
                }
            }
        } else {
            // `out`: the actual has the type the formal part converts to.
            let types = self.conv_types(scope, e, k, a);
            let c = self.cands(scope, x);
            let t = types
                .iter()
                .filter_map(|&t| self.best_cost(&c, Expected::Type(t)).map(|n| (n, t)))
                .min_by_key(|x| x.0)
                .map(|x| x.1)
                .or_else(|| match &c {
                    CandSet::Types(is) if is.len() == 1 => Some(is[0].ty),
                    _ => None,
                })
                .unwrap_or(self.base(st));
            let id = self.resolve_in(scope, x, self.types[t].first_subtype);
            if self.is_poison_expr(id) {
                return None;
            }
            if named
                && self.actual_conversion(id).is_some()
                && !self.actual_conv_allowed(class, &mode, name, x.span)
            {
                return None;
            }
            (id, None, None)
        };
        if !self.check_actual_class(class, &mode, name, obj) {
            return None;
        }
        // The way out: the formal part, naming the formal, of the actual's type.
        let want = self.base(self.exprs[obj].subtype);
        let poison = self.poison.ty;
        let fe = self.in_formal_region(scope, p, |db, fs| {
            let CandSet::Types(is) = db.name_cands_uncached(fs, fname) else {
                return Err(false);
            };
            if is.iter().any(|i| i.ty == poison) {
                return Err(true);
            }
            // Overloads told apart only by the conversion's result type are ambiguous
            // (6.5.7.1; user, 2026-10-19, majority rule): the actual does not choose.
            let mut types: Vec<TypeId> = Vec::new();
            for i in &is {
                if !types.contains(&i.ty) {
                    types.push(i.ty);
                }
            }
            if types.len() > 1 {
                db.report(
                    Diagnostic::error(
                        diag::AMBIGUOUS,
                        format!(
                            "the formal conversion `{}` is ambiguous: its overloads differ only in their result type",
                            db.source_text(fname.span)
                        ),
                    )
                    .with_primary(fname.span, "several conversions fit the formal")
                    .with_help("qualify the conversion or give the overloads different parameter types"),
                );
                return Err(true);
            }
            let mut fit: Vec<Interp> = Vec::new();
            for i in is {
                if i.ty == want && !fit.iter().any(|f| f.via == i.via) {
                    fit.push(i);
                }
            }
            match fit[..] {
                [i] => Ok(db.build_name(fs, fname, i, fname.span)),
                _ => Err(false),
            }
        });
        let fe = match fe {
            Ok(fe) if self.is_poison_expr(fe) => return None,
            Ok(fe) => {
                // The formal part names the formal, it reads nothing.
                self.not_read(fe);
                fe
            }
            Err(true) => return None,
            Err(false) => {
                let tn = self.type_name(want);
                self.report(
                    Diagnostic::error(
                        diag::NO_OVERLOAD,
                        format!(
                            "no conversion of formal `{}` gives the actual's type `{tn}`",
                            name.text
                        ),
                    )
                    .with_primary(fname.span, format!("expected a conversion to `{tn}`"))
                    .with_help("use a function of one parameter of the formal's type, or a type conversion"),
                );
                return None;
            }
        };
        let Some((formal_conv, _)) = self.actual_conversion(fe) else {
            return fail(
                self,
                fname.span,
                format!(
                    "`{}` is not a conversion of formal `{}`",
                    self.source_text(fname.span),
                    name.text
                ),
                "write a conversion function call or a type conversion of the formal",
            );
        };
        // A formal that is not fully constrained takes its bounds from the conversion
        // (5.3.2.2, as in port maps; user, 2026-10-19): the actual part's result subtype
        // for `inout`, the formal part's type mark or parameter subtype for `out`.
        if self.constrainedness(st) != crate::types::Constrainedness::Full {
            let def = match copy_in {
                Some(c) => Some(self.exprs[c].subtype),
                None => self.formal_conv_subtype(fe),
            };
            if def.is_none_or(|d| self.constrainedness(d) != crate::types::Constrainedness::Full) {
                self.report(
                    Diagnostic::error(
                        diag::FORMAL_CONV_UNCONSTRAINED,
                        format!(
                            "the conversion of unconstrained formal `{}` must give its bounds",
                            name.text
                        ),
                    )
                    .with_primary(
                        fname.span,
                        format!(
                            "`{}` is of the unconstrained subtype `{}`",
                            name.text,
                            self.subtype_name(st)
                        ),
                    )
                    .with_help("use a constrained subtype as the type mark, or a function with a constrained parameter subtype"),
                );
                return None;
            }
        }
        Some(Actual::Converted {
            formal_conv: Some(formal_conv),
            actual_conv,
            actual: obj,
            copy_in,
            copy_out: Some(fe),
        })
    }

    /// 4.2.2.1 (E1360): the actual of a variable, signal or file parameter is a name of an
    /// object of that class; a constant parameter takes any expression. `false` when reported.
    fn check_actual_class(
        &mut self,
        class: crate::decl::ObjClass,
        mode: &crate::hir::Mode,
        formal: Ident,
        id: ExprId,
    ) -> bool {
        use crate::decl::ObjClass as C;
        use crate::hir::ObjRoot;
        use crate::lsp::NameRoot;
        if class == C::Constant || self.is_poison_expr(id) {
            return true;
        }
        // An actual conversion of an object name is still that object (6.5.7.1).
        let mut e = id;
        while let Some((_, operand)) = self.actual_conversion(e) {
            e = operand;
        }
        let (actual, name) = match self.name_root(e) {
            None => (None, None),
            Some(NameRoot::Deref) => (Some(C::Variable), None),
            Some(NameRoot::Obj(ObjRoot::ImplicitSignal(d))) => (Some(C::Signal), Some(d)),
            Some(NameRoot::Obj(ObjRoot::External(x))) => (
                Some(match self.externals[x].class {
                    ast::ExternalClass::Constant => C::Constant,
                    ast::ExternalClass::Signal => C::Signal,
                    ast::ExternalClass::Variable => C::Variable,
                }),
                None,
            ),
            Some(NameRoot::Obj(ObjRoot::Decl(d))) => match &self.decls[d].kind {
                DeclKind::Object(o) => (Some(o.class), Some(d)),
                DeclKind::Error => return true,
                _ => (None, None),
            },
        };
        let fits = matches!(
            (class, actual),
            (C::Variable, Some(C::Variable | C::SharedVariable))
                | (C::Signal, Some(C::Signal))
                | (C::File, Some(C::File))
        );
        if fits {
            // 4.2.2.3: the actual of a signal parameter is a static signal name (#813).
            if class == C::Signal
                && let Some((_, _, s, cut)) = self.lsp_parts(e)
                && (cut || s == Staticness::None)
            {
                // `--relaxed` (R6): accepted with a warning, as by sim1's relaxed switch.
                if self.relaxed() {
                    self.report(
                        Diagnostic::warning(
                            diag::W_SIGNAL_ACTUAL_NOT_STATIC,
                            format!(
                                "the actual of signal parameter `{}` is not a static name",
                                formal.text
                            ),
                        )
                        .with_primary(self.exprs[id].span, "accepted by `--relaxed`")
                        .with_note("LRM 4.2.2.3 needs a static signal name here"),
                    );
                    return true;
                }
                self.report(
                    Diagnostic::error(
                        diag::CLASS_MISUSE,
                        format!(
                            "the actual of signal parameter `{}` must be a static name",
                            formal.text
                        ),
                    )
                    .with_primary(self.exprs[id].span, "not a static name")
                    .with_note("LRM 4.2.2.3: every index and slice of the actual of a signal parameter is a static expression"),
                );
                return false;
            }
            return true;
        }
        let word = |c: C| match c {
            C::Constant => "constant",
            C::Signal => "signal",
            C::Variable => "variable",
            C::SharedVariable => "shared variable",
            C::File => "file",
        };
        let target = match (class, mode) {
            (C::Variable, crate::hir::Mode::Out) => "`out` parameter".to_string(),
            (C::Variable, crate::hir::Mode::Inout) => "`inout` parameter".to_string(),
            _ => format!("{} parameter", word(class)),
        };
        let what = match (actual, name) {
            (Some(c), Some(d)) => format!("{} `{}`", word(c), self.decls[d].name),
            (Some(c), None) => word(c).to_string(),
            (None, _) => "an expression".to_string(),
        };
        let span = self.exprs[id].span;
        self.report(
            Diagnostic::error(
                diag::CLASS_MISUSE,
                format!("{what} cannot be the actual for {target} `{}`", formal.text),
            )
            .with_primary(span, format!("not a {}", word(class)))
            .with_help(format!("pass the name of a {}", word(class))),
        );
        false
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
        let from = self.base(self.exprs[operand].subtype);
        let to = self.base(st);
        if !self.is_poison_expr(operand) && !self.closely_related(from, to) {
            let (f, t) = (self.type_name(from), self.type_name(to));
            self.report(
                Diagnostic::error(
                    diag::BAD_CONVERSION,
                    format!("cannot convert `{f}` to `{t}`"),
                )
                .with_primary(span, "these types are not closely related")
                .with_note(
                    "9.3.6: only abstract numeric types, array types with the same \
                     dimensionality and closely related element types, and (2019) records with \
                     the same element names convert to each other",
                ),
            );
            return self.poison.expr;
        }
        if !self.is_poison_expr(operand) {
            self.warn_conversion_bounds(operand, st);
        }
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
            if self.warn_if_outside(id, st, span) {
                self.exprs[id].value = None;
            }
        }
        id
    }

    /// `W1403` (9.3.6): `operand` has static bounds and converts to the unconstrained array
    /// subtype `st`, whose index subtypes do not contain them. The result takes the operand's
    /// bounds, so the conversion fails when it executes; like the scalar case (L51) the code
    /// may be dead, so it stays a warning and the run-time check fails.
    fn warn_conversion_bounds(&mut self, operand: ExprId, st: SubtypeId) {
        use crate::expr::aggregate::subtype_range;
        let TypeKind::Array { indexes, .. } = &self.types[self.base(st)].kind else {
            return;
        };
        if matches!(
            &self.subtypes[st].constraint,
            crate::types::Constraint::Array { dims: Some(_), .. }
        ) {
            return;
        }
        let os = self.exprs[operand].subtype;
        for (k, &ix) in indexes.clone().iter().enumerate() {
            let (Some(r), Some(o)) = (subtype_range(self, os, k), subtype_range(self, ix, 0))
            else {
                continue;
            };
            if r.is_null() || o.is_null() || (o.contains(r.low()) && o.contains(r.high())) {
                continue;
            }
            let (name, span) = (self.subtype_name(ix), self.exprs[operand].span);
            self.report(
                Diagnostic::warning(
                    diag::W_STATIC_OUTSIDE,
                    format!("the bounds of the operand are not inside index subtype `{name}` of the target"),
                )
                .with_primary(
                    span,
                    format!("{} to {} is outside {} to {}", r.low(), r.high(), o.low(), o.high()),
                )
                .with_note("the code may never run, so this is checked again at run time (LRM 9.3.6): the result has the operand's bounds, which must belong to the index subtypes of the target"),
            );
            return;
        }
    }

    /// 9.3.6: a type is closely related to itself; abstract numeric types to each other;
    /// array types with the same dimensionality and closely related element types; in 2019
    /// record types with the same element names whose types are closely related.
    /// Incomplete, generic and poison types are not checked here.
    pub(crate) fn closely_related(&self, a: TypeId, b: TypeId) -> bool {
        use crate::types::TypeKind as K;
        let numeric = |k: &K| {
            matches!(
                k,
                K::Integer | K::Floating | K::UniversalInteger | K::UniversalReal
            )
        };
        let (ka, kb) = (&self.types[a].kind, &self.types[b].kind);
        match (ka, kb) {
            _ if a == b => true,
            (K::Error | K::Incomplete { .. } | K::Generic { .. }, _)
            | (_, K::Error | K::Incomplete { .. } | K::Generic { .. }) => true,
            (
                K::Array {
                    indexes: ia,
                    elem: ea,
                },
                K::Array {
                    indexes: ib,
                    elem: eb,
                },
            ) => {
                ia.len() == ib.len()
                    && self.closely_related(self.base(*ea), self.base(*eb))
                    // 1993 7.3.5: closely related index types and the same element type (#813).
                    && (self.file_rev() >= crate::LangRev::Vhdl2008
                        || (self.base(*ea) == self.base(*eb)
                            && ia.iter().zip(ib.iter()).all(|(x, y)| {
                                self.closely_related(self.base(*x), self.base(*y))
                            })))
            }
            // 2019 (LCS-2016-075): records with the same element names, closely related types.
            (K::Record { elems: ra }, K::Record { elems: rb })
                if self.file_rev() >= crate::LangRev::Vhdl2019 =>
            {
                ra.len() == rb.len()
                    && ra.iter().all(|x| {
                        rb.iter().any(|y| {
                            x.name.key == y.name.key
                                && self.closely_related(self.base(x.subtype), self.base(y.subtype))
                        })
                    })
            }
            _ => numeric(ka) && numeric(kb),
        }
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
        if args.is_empty()
            && let Some((p, a, dim)) = Self::range_attr_alias(prefix, attr)
        {
            return self.build_attr(scope, p, a, dim, i, span);
        }
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return self.build_user_attr(scope, prefix, attr, span);
        };
        if a == PredefAttr::Value
            && args.is_empty()
            && let Some(r) = self.range_attr(scope, prefix)
        {
            return match r {
                Ok((
                    crate::hir::RangeC::Attr {
                        prefix: p,
                        reverse,
                        dim,
                    },
                    t,
                )) => self.range_value(p, reverse, dim, t, span),
                _ => self.poison_expr(),
            };
        }
        let (aprefix, mut stat, arg_t) = match i.via {
            Via::AttrOf(t) if t == self.poison.ty => {
                // 'simple_name, 'path_name, 'instance_name of a named entity.
                let d = match self.classify(scope, prefix) {
                    NameRes::One(d) => d,
                    NameRes::Overloads(ds) => ds[0],
                    r => return Expr::poison_reported(self, scope, r, prefix.span),
                };
                let stat = if a == PredefAttr::SimpleName {
                    Staticness::Locally
                } else {
                    Staticness::Globally
                };
                (AttrPrefix::Entity(d), stat, None)
            }
            Via::AttrOf(t) => {
                let p = self.resolve_prefix(scope, prefix, t);
                if !a.reads_value() {
                    self.not_read(p);
                }
                // 6.5.2: a `linkage` port is read only as the actual of a `linkage` formal,
                // so not through an attribute of its value either (#813).
                if a.reads_value()
                    && let Some(crate::hir::ObjRoot::Decl(d)) = self.lsp(p).map(|l| l.root)
                    && matches!(&self.decls[d].kind, DeclKind::Object(o) if o.mode == Some(crate::hir::Mode::Linkage))
                {
                    self.report(
                        Diagnostic::error(
                            diag::CLASS_MISUSE,
                            format!(
                                "cannot read `'{}` of port `{}` of mode `linkage`",
                                attr.text, self.decls[d].name
                            ),
                        )
                        .with_primary(prefix.span, "a `linkage` port")
                        .with_note("a `linkage` port can only be associated with another `linkage` port (LRM 6.5.2)"),
                    );
                    return self.poison_expr();
                }
                // 4.2.2.2: of a formal signal parameter these attributes are not allowed;
                // of another signal they are, and the implicit signal lives outside the
                // subprogram (sim2, sim3).
                if matches!(
                    a,
                    PredefAttr::Delayed
                        | PredefAttr::Stable
                        | PredefAttr::Quiet
                        | PredefAttr::Transaction
                ) && self.in_subprogram(scope)
                    && let Some(crate::hir::ObjRoot::Decl(d)) = self.lsp(p).map(|l| l.root)
                    && matches!(&self.decls[d].kind, DeclKind::Object(o) if o.role == crate::decl::ObjRole::Param)
                {
                    self.report(
                        Diagnostic::error(
                            diag::SIGNAL_ATTR_IN_SUBPROGRAM,
                            format!(
                                "implicit signal attribute `{}` of formal signal parameter `{}` cannot be read in a subprogram",
                                attr.text, self.decls[d].name
                            ),
                        )
                        .with_primary(prefix.span, "a formal signal parameter")
                        .with_note("LRM 4.2.2.2: `'stable`, `'quiet`, `'transaction` and `'delayed` of a formal signal parameter are not allowed"),
                    );
                    return self.poison_expr();
                }
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
                        let st = self.exprs[p].subtype;
                        if self.array_locally_static(st) {
                            // An object of a locally static subtype (9.4.2 e).
                            Staticness::Locally
                        } else if self.constrainedness(st) == crate::types::Constrainedness::Full
                            || self.is_port_or_generic(p)
                        {
                            // A port or generic gets its bounds once, at elaboration: like sim1,
                            // sim2 and sim3, its array attributes are globally static (9.4.3).
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
        if is_array
            && matches!(a.info().param, attr::Param::Dim)
            && let Some(&x) = hargs.first()
            && self.check_dimension(x, t).is_err()
        {
            return self.poison_expr();
        }
        let subtype = self.types[i.ty].first_subtype;
        // `s'delayed(t)`, `s'stable(t)`, `s'quiet(t)`, `s'transaction` declare an implicit
        // signal (16.2.4, sem.md 6.3); the name reads it.
        if let (
            PredefAttr::Delayed | PredefAttr::Stable | PredefAttr::Quiet | PredefAttr::Transaction,
            AttrPrefix::Expr(p),
        ) = (a, aprefix)
        {
            let st = if a == PredefAttr::Delayed {
                self.exprs[p].subtype
            } else {
                subtype
            };
            // The implicit signal belongs to the enclosing concurrent region or process, never
            // to a subprogram, loop or sequential block.
            let mut region = scope;
            while (self.in_subprogram(region)
                || matches!(
                    self.scopes[region].kind,
                    crate::scope::ScopeKind::Loop | crate::scope::ScopeKind::SeqBlock
                ))
                && let Some(up) = self.scopes[region].parent
            {
                region = up;
            }
            return match crate::attr::implicit_signal(
                self,
                a,
                false,
                region,
                p,
                hargs.first().copied(),
                st,
                span,
            ) {
                Ok(d) => self.object_ref(d, span),
                Err(_) => self.poison_expr(),
            };
        }
        match aprefix {
            AttrPrefix::Subtype(st) if !is_array => self.integer_bound_changed(a, st, span),
            AttrPrefix::Expr(p) if !is_array => {
                self.integer_bound_changed(a, self.exprs[p].subtype, span)
            }
            _ => {}
        }
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

    /// 2019 `R'value` (16.2.6) for `R` = `p'range(dim)` (or `'reverse_range`): the aggregate
    /// `(left, right, direction)` of the range, built from `'left`, `'right` and `'ascending`.
    fn range_value(
        &mut self,
        p: AttrPrefix,
        reverse: bool,
        dim: u32,
        t: TypeId,
        span: Span,
    ) -> ExprId {
        let Ok(rec) = self.range_record_of(t, span) else {
            return self.poison_expr();
        };
        let TypeKind::Record { elems } = &self.types[self.base(rec)].kind else {
            return self.poison_expr();
        };
        let (el, dir) = (elems[0].subtype, elems[2].subtype);
        // A prefix with a static range gives a constant record (usable as a range, 099).
        let pst = match p {
            AttrPrefix::Subtype(st) => Some(st),
            AttrPrefix::Expr(e) => Some(self.exprs[e].subtype),
            AttrPrefix::Entity(_) => None,
        };
        let is_array = |db: &Self, st| matches!(db.types[db.base(st)].kind, TypeKind::Array { .. });
        if let Some(st) = pst
            && let Some(r) = crate::expr::aggregate::subtype_range(
                self,
                st,
                if is_array(self, st) {
                    dim as usize - 1
                } else {
                    0
                },
            )
        {
            let enum_ = matches!(self.types[t].kind, TypeKind::Enum { .. });
            let mk = |v: i64| {
                if enum_ {
                    ConstValue::Enum(v as u32)
                } else {
                    ConstValue::Int(v)
                }
            };
            let (l, r_, asc) = if reverse {
                (r.right, r.left, r.dir != Direction::To)
            } else {
                (r.left, r.right, r.dir == Direction::To)
            };
            let v =
                ConstValue::Record(Box::new([mk(l), mk(r_), ConstValue::Enum(u32::from(!asc))]));
            return self.const_expr(v, rec, span);
        }
        let stat = match p {
            AttrPrefix::Subtype(st) => self.subtype_staticness(st),
            _ => Staticness::None,
        };
        let uint =
            self.types[self.well_known.universal_integer.unwrap_or(self.poison.ty)].first_subtype;
        let boolean = self.types[self.well_known.boolean.unwrap_or(self.poison.ty)].first_subtype;
        let attr = |db: &mut Self, a, st| {
            let args: Box<[ExprId]> = if dim > 1 {
                Box::new([db.const_expr(ConstValue::Int(dim.into()), uint, span)])
            } else {
                Box::new([])
            };
            db.add_expr(Expr {
                kind: ExprKind::Attr {
                    attr: a,
                    prefix: p,
                    args,
                },
                subtype: st,
                span,
                stat,
                value: None,
            })
        };
        let (l, r) = if reverse {
            (PredefAttr::Right, PredefAttr::Left)
        } else {
            (PredefAttr::Left, PredefAttr::Right)
        };
        let left = attr(self, l, el);
        let right = attr(self, r, el);
        let asc = attr(self, PredefAttr::Ascending, boolean);
        // ASCENDING and DESCENDING are positions 0 and 1 of RANGE_DIRECTION.
        let (first, second) = if reverse { (1, 0) } else { (0, 1) };
        let a = self.const_expr(ConstValue::Enum(first), dir, span);
        let b = self.const_expr(ConstValue::Enum(second), dir, span);
        let direction = self.add_expr(Expr {
            kind: ExprKind::Conditional {
                alts: Box::new([(a, Some(asc)), (b, None)]),
            },
            subtype: dir,
            span,
            stat,
            value: None,
        });
        let assocs = [left, right, direction]
            .into_iter()
            .enumerate()
            .map(|(i, value)| crate::hir::AggAssoc {
                choices: Box::new([crate::hir::Choice::Element(i as u32)]),
                value,
                slice_piece: false,
            })
            .collect();
        self.add_expr(Expr {
            kind: ExprKind::Aggregate {
                assocs,
                bounds: crate::hir::AggBounds::Record,
            },
            subtype: rec,
            span,
            stat,
            value: None,
        })
    }

    /// Report why attribute name `prefix'attr` has no interpretation (E1621,
    /// E1007 through [`attr::select`]). Returns `false` when it found nothing
    /// to report.
    pub(crate) fn report_attr(&mut self, scope: ScopeId, prefix: &ast::Name, attr: Ident) -> bool {
        let ps = match self.attr_prefixes(scope, prefix) {
            Ok(ps) => ps,
            Err(r) => {
                self.report_name(scope, r, prefix.span, "a name with attributes");
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
                && let Some(h) = crate::suggest::suggest(
                    attr.key.as_str(),
                    "attribute",
                    attr::TABLE.iter().map(|i| (i.name, i.name)),
                )
            {
                d = d.with_help(h);
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

    /// The object an object alias `d` names as a whole (through further aliases), `None`
    /// when `d` is not such an alias: a part of an object (`alias a : bit is v(1)`) is
    /// another entity.
    fn aliased_object(&self, d: DeclId) -> Option<DeclId> {
        let mut d = d;
        let mut hops = 0;
        while let DeclKind::Alias(Alias::Object { target, .. }) = &self.decls[d].kind {
            let ExprKind::Obj(next) = self.exprs[*target].kind else {
                return None;
            };
            d = next;
            hops += 1;
            if hops > 64 {
                return None;
            }
        }
        (hops > 0).then_some(d)
    }

    /// The value of user attribute `key` of the named entity `d` denotes (7.2): its own,
    /// else that of the object it is an alias of (an attribute of an object applies to every
    /// alias of the whole object, 4.3.3).
    fn user_attr_value(&self, d: DeclId, key: Symbol) -> Option<crate::ids::AttrValueId> {
        attr::user_value(self, self.entity(d), key).or_else(|| {
            let root = self.aliased_object(d)?;
            attr::user_value(self, root, key)
        })
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
            PSrc::Entity(d) | PSrc::Object(d, _) => self.user_attr_value(d, attr.key),
            PSrc::Type(_) => {
                let d = self.type_mark_decl(scope, prefix)?;
                self.user_attr_value(d, attr.key)
            }
            PSrc::Value(_) => None,
        });
        let Some(v) = found else {
            return self.poison_expr();
        };
        let value = self.attr_values[v].value;
        let subtype =
            attr::spec_subtype(self, self.attr_values[v].attr).unwrap_or(self.exprs[value].subtype);
        let (stat, folded) = (self.exprs[value].stat, self.exprs[value].value);
        // The value takes the attribute's subtype, bounds included (7.2, #638).
        let folded = folded.and_then(|c| self.rebound(c, subtype));
        self.add_expr(Expr {
            kind: ExprKind::AttrValue { spec: v },
            subtype,
            span,
            stat,
            value: folded,
        })
    }

    /// `W1404` (sem.md 4.3): a static index or slice outside a static index range. Only a
    /// warning, the code may be dead; the run-time check stays.
    fn warn_static_index(&mut self, span: Span, msg: String) {
        self.report(
            Diagnostic::warning(diag::W_STATIC_INDEX, msg)
                .with_primary(span, "outside the index range")
                .with_note(
                    "the code may never run, so this is checked again at run time (LRM 8.4, 8.5)",
                ),
        );
    }

    /// Value `v` of discrete type `t` as VHDL source: a literal name or a number.
    fn index_image(&self, t: TypeId, v: i64) -> String {
        match &self.types[t].kind {
            TypeKind::Enum { lits } => match usize::try_from(v).ok().and_then(|i| lits.get(i)) {
                Some(&d) => self.decls[d].name.to_string(),
                None => v.to_string(),
            },
            _ => v.to_string(),
        }
    }

    fn range_image(&self, t: TypeId, r: crate::expr::aggregate::StaticRange) -> String {
        let dir = match r.dir {
            ast::Direction::To => "to",
            ast::Direction::Downto => "downto",
        };
        format!(
            "{} {dir} {}",
            self.index_image(t, r.left),
            self.index_image(t, r.right)
        )
    }

    /// The dimension argument `x` of an array attribute of array type `t` (16.2.3):
    /// a locally static positive integer, at most the number of dimensions.
    fn check_dimension(&mut self, x: ExprId, t: TypeId) -> Result<(), Reported> {
        if self.is_poison_expr(x) {
            return Err(Reported);
        }
        let span = self.exprs[x].span;
        let v = if self.exprs[x].stat == Staticness::Locally {
            self.value(x).and_then(|v| v.as_int())
        } else {
            None
        };
        let Some(d) = v else {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_STATIC,
                    "the dimension of an array attribute must be a locally static expression",
                )
                .with_primary(span, "not locally static"),
            ));
        };
        let n = match &self.types[t].kind {
            TypeKind::Array { indexes, .. } => indexes.len(),
            _ => return Ok(()),
        };
        if d < 1 || d as u64 > n as u64 {
            return Err(self.error(
                Diagnostic::error(
                    diag::ATTR_DIMENSION,
                    format!("dimension {d} does not exist for a {n}-dimensional array"),
                )
                .with_primary(span, "no such dimension"),
            ));
        }
        Ok(())
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

    /// 9.4.2: a call of a function of `std_logic_1164`, `numeric_bit`, `numeric_std`,
    /// `numeric_bit_unsigned` or `numeric_std_unsigned` with locally static actuals is
    /// locally static.
    fn in_static_ieee_package(&self, f: DeclId) -> bool {
        let Some(pkg) = self.scopes[self.decls[f].scope].owner else {
            return false;
        };
        let DeclKind::Unit(u) = self.decls[pkg].kind else {
            return false;
        };
        self.libs[self.units[u].lib].name.key.as_str() == "ieee"
            && matches!(
                self.decls[pkg].name.key().as_str(),
                "std_logic_1164"
                    | "numeric_bit"
                    | "numeric_std"
                    | "numeric_bit_unsigned"
                    | "numeric_std_unsigned"
            )
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

    /// Staticness of the constraint a value is held to: a scalar range's, or a constrained
    /// array's index ranges (a value's fold must not skip the run-time check against a
    /// subtype whose bounds are known only at elaboration, 5.3.2.2, 9.3.3.2).
    pub(crate) fn constraint_staticness(&self, st: SubtypeId) -> Staticness {
        if matches!(
            self.subtypes[st].constraint,
            crate::types::Constraint::Array { dims: Some(_), .. }
        ) && !self.array_locally_static(st)
        {
            return Staticness::Globally;
        }
        self.subtype_staticness(st)
    }

    /// An array subtype whose index ranges are all locally static (9.4.2).
    fn array_locally_static(&self, st: SubtypeId) -> bool {
        let crate::types::Constraint::Array {
            dims: Some(dims), ..
        } = &self.subtypes[st].constraint
        else {
            return false;
        };
        dims.iter().all(|r| match r {
            crate::types::RangeC::Explicit { left, right, .. } => {
                self.exprs[*left].stat.min(self.exprs[*right].stat) == Staticness::Locally
            }
            crate::types::RangeC::Attr { .. } => false,
        })
    }

    /// The expression names a port or generic of the enclosing design entity,
    /// or a sub-element of one (`bus.dat`: its bounds are fixed at elaboration too).
    fn is_port_or_generic(&self, e: ExprId) -> bool {
        match self.lsp(e).map(|l| l.root) {
            Some(crate::hir::ObjRoot::Decl(d)) => matches!(
                &self.decls[d].kind,
                DeclKind::Object(o) if matches!(o.role, crate::decl::ObjRole::Port | crate::decl::ObjRole::Generic)
            ),
            _ => false,
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
    fn poison_reported(db: &mut Db, scope: ScopeId, r: NameRes, span: Span) -> ExprId {
        db.report_name(scope, r, span, "a named entity");
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

/// Why an association list does not fit a formal list (6.5.7.1), with the index of the
/// offending association.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssocErr {
    /// A positional association after the last formal.
    TooMany(usize),
    /// A named association whose formal is not a parameter.
    Unknown(usize),
    /// A named association of a formal associated before.
    Twice(usize),
    /// Positional after named, or a formal designator rooted in no formal (a conversion).
    Other,
}

/// The associations of one formal (6.5.7.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    None,
    /// Associated as a whole by this association.
    Whole(usize),
    /// Associated individually by these associations: `f(0) => a, f(1) => b`.
    Parts(Vec<usize>),
    /// Associated as a whole by this association, through a formal conversion: `conv(f) => a`.
    Conv(usize),
}

/// The formal a formal conversion `conv(f)` converts, if `n` has that form (6.5.7.1).
pub(crate) fn conv_formal(n: &ast::Name) -> Option<Ident> {
    let ast::NameKind::Apply { args, .. } = &n.kind else {
        return None;
    };
    let [
        ast::AssocElem {
            formal: None,
            actual: ast::Actual::Expr(e),
            ..
        },
    ] = &args[..]
    else {
        return None;
    };
    match &e.kind {
        ast::ExprKind::Name(ast::Name {
            kind: ast::NameKind::Simple(i),
            ..
        }) => Some(*i),
        _ => None,
    }
}

/// The formal a formal designator names: `f` in `f`, `f(0)`, `f.e`, `f(1 to 3)`.
pub(crate) fn formal_root(n: &ast::Name) -> Option<Ident> {
    match &n.kind {
        ast::NameKind::Simple(i) => Some(*i),
        ast::NameKind::Apply { prefix, .. } | ast::NameKind::Selected { prefix, .. } => {
            formal_root(prefix)
        }
        _ => None,
    }
}

/// Map formals (by key) to their associations: positional first, then named, a formal
/// either whole, by sub-elements or through a conversion (6.5.7.1). `Err` if the
/// association does not fit (too many, unknown or repeated name). A designator rooted in a
/// formal is a sub-element (`f(g)` with formals `f` and `g`), as in port maps.
pub(crate) fn assoc_map(
    formals: &[Symbol],
    args: &[ast::AssocElem],
) -> Result<smallvec::SmallVec<[Slot; 8]>, AssocErr> {
    let mut map = smallvec::SmallVec::from_elem(Slot::None, formals.len());
    let mut named = false;
    for (k, a) in args.iter().enumerate() {
        match &a.formal {
            None => {
                if named {
                    return Err(AssocErr::Other);
                }
                if k >= formals.len() {
                    return Err(AssocErr::TooMany(k));
                }
                map[k] = Slot::Whole(k);
            }
            Some(f) => {
                named = true;
                let whole = matches!(f.kind, ast::NameKind::Simple(_));
                let i = formal_root(f).ok_or(AssocErr::Other)?;
                let find = |key| formals.iter().position(|&x| x == key);
                let (pos, conv) = match find(i.key) {
                    Some(p) => (p, false),
                    None if whole => return Err(AssocErr::Unknown(k)),
                    None => match conv_formal(f).and_then(|c| find(c.key)) {
                        Some(p) => (p, true),
                        None => return Err(AssocErr::Other),
                    },
                };
                match (&mut map[pos], whole) {
                    (s @ Slot::None, _) if conv => *s = Slot::Conv(k),
                    _ if conv => return Err(AssocErr::Twice(k)),
                    (s @ Slot::None, true) => *s = Slot::Whole(k),
                    (s @ Slot::None, false) => *s = Slot::Parts(vec![k]),
                    (Slot::Parts(v), false) => v.push(k),
                    _ => return Err(AssocErr::Twice(k)),
                }
            }
        }
    }
    Ok(map)
}

/// Whether `attr(args)` is an attribute call. `x'subtype`, `x'element` and `t'base` denote
/// subtypes, so applying them is a type conversion; a user-defined attribute has a value that
/// the arguments index (`x'a(1)`, LRM 7.2). A predefined attribute without a parameter takes
/// none either: `x'simple_name(1)` indexes the string value (#653).
fn takes_args(attr: &ast::Ident) -> bool {
    if matches!(attr.key.as_str(), "subtype" | "element" | "base") {
        return false;
    }
    PredefAttr::from_key(attr.key).is_some_and(|a| a.info().param != attr::Param::None)
}
